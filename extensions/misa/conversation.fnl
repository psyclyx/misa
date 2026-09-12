;; Durable conversation log. Canonical history stays owned by `misa.agent`; this
;; owner journals it when a turn completes and can put it back on request.

(fn settings [configuration]
  "Read conversation settings with their defaults."
  (let [value (or configuration.conversation {})]
    {:enabled (not= value.enabled false)
     :id (and (= (type value.id) :string) value.id)
     :label (and (= (type value.label) :string) value.label)
     :list_limit (or value.list_limit 25)}))

(fn session-id [config cofx]
  "Name this session's conversation: configured, or derived from its clock."
  (or config.id (.. :session- (tostring cofx.clock.wall_ms))))

(fn start [db _ cofx]
  "Name this session's conversation and reset its sync cursor."
  (let [config (settings cofx.config)]
    (when config.enabled
      (let [id (session-id config cofx)
            initial {: id
                     :label config.label
                     :list_limit config.list_limit
                     :synced 0}]
        (assert (id:match "^[%w._-]+$")
                "conversation.id may only contain letters, digits, dot, dash, or underscore")
        {:patch {:conversation (misa.replace initial)}}))))

(fn label-of [messages]
  "Summarize a conversation from its first user message."
  (var found nil)
  (each [_ message (ipairs messages) &until found]
    (each [_ block (ipairs (or message.content [])) &until found]
      (when (and (= message.role :user) (= block.type :text)
                 (= (type block.text) :string) (not= block.text ""))
        (set found (: (block.text:gsub "%s+" " ") :sub 1 60)))))
  found)

;; One turn's canonical history can be longer than a single append accepts: the
;; session rejects an effect carrying more than 256 entries, and the store
;; refuses a batch whose encoded message payloads total more than 4 MiB. The
;; journal therefore sends one bounded append at a time and continues from each
;; completion, so a long turn is recorded in order instead of failing whole.
(local max-append-entries 256)
(local max-append-bytes (* 4 1024 1024))

(fn message-entry [message]
  "One journal entry for a canonical message."
  {:kind :message :data message})

(fn reset-entry []
  "The marker that starts a replaced branch over in the log."
  {:kind :reset :data {:replaced true}})

(fn json-encode []
  "The installed JSON encoder, when this application has the JSON service."
  (let [json (. misa :json)]
    (if (= (type json) :table)
        (let [encode (. json :encode)]
          (if (= (type encode) :function) encode)))))

(fn payload-bytes [message]
  "Encoded size of one journaled message, as the store measures it."
  (let [encode (json-encode)]
    (if encode (length (encode message)) 0)))

(fn bounded-batch [messages from]
  "The next append of messages past FROM: `{:entries ... :last n}`, where LAST
   is the message index the append brings the log up to. One message is always
   taken even when it alone exceeds the byte budget, because the store, not the
   journal, decides what one message may contain."
  (let [total (length messages)
        entries []]
    (var index (+ from 1))
    (var bytes 0)
    (var full false)
    (while (and (<= index total) (< (length entries) max-append-entries)
                (not full))
      (let [message (. messages index)
            size (payload-bytes message)]
        (if (or (= (length entries) 0) (<= (+ bytes size) max-append-bytes))
            (do
              (table.insert entries (message-entry message))
              (set bytes (+ bytes size))
              (set index (+ index 1)))
            (set full true))))
    {: entries :last (+ from (length entries))}))

(fn append-effect [conversation entries label last]
  "One bounded journal append, named by the history it covers."
  {:type :conversation/append
   : conversation
   : entries
   :metadata (when label {: label})
   :completion :conversation/appended
   :id (.. conversation "/" (tostring last))})

(fn next-append [conversation messages synced]
  "The next append of this journal, or nil when it has caught up. A shorter
   history means the branch was replaced, so its reset marker is sent first and
   the new branch is journaled from the beginning afterwards."
  (let [count (length messages)
        id conversation.id
        label (or conversation.label (label-of messages))]
    (if (< count synced)
        {:patch {:conversation {: label :pending {:kind :reset} :synced 0}}
         :fx [(append-effect id [(reset-entry)] label 0)]}
        (let [{: entries : last} (bounded-batch messages synced)]
          (when (> (length entries) 0)
            {:patch {:conversation {: label :pending {:kind :messages : last}}}
             :fx [(append-effect id entries label last)]})))))

(fn journal [db]
  "Append canonical history that the durable log has not seen yet. An append in
   flight already covers what a later turn would add, so a turn that settles
   meanwhile waits for that chain instead of writing it twice."
  (let [conversation (or db.conversation {})
        id conversation.id
        messages (or (and db.agent db.agent.messages) [])]
    (when (and (= (type id) :string) (> (length messages) 0)
               (not conversation.pending))
      (next-append conversation messages (or conversation.synced 0)))))

(fn appended [db event]
  "Continue the journal after an append settles. The cursor advances to the
   history that append covered, so an unrecordable range is reported and skipped
   once rather than retried by every later turn."
  (let [conversation (or db.conversation {})
        pending conversation.pending]
    (when pending
      (let [synced (if (= (. pending :kind) :messages) (. pending :last) 0)
            settled {:conversation {: synced :pending misa.delete}}
            messages (or (and db.agent db.agent.messages) [])]
        (if (not event.ok)
            {:patch settled
             :fx [{:type :dispatch
                   :event {:level :error
                           :text (.. "Could not record the conversation: "
                                     (tostring (or event.message :unknown)))
                           :type :transcript/harness}}]}
            (let [next (and (> (length messages) 0)
                            (next-append (misa.patch conversation settled)
                                         messages synced))]
              (if next
                  {:patch (misa.patch settled next.patch) :fx next.fx}
                  {:patch settled})))))))

(fn completed [db _]
  "Journal canonical history once a turn has settled."
  (journal db))

(fn text-of [content]
  "Join the text blocks of a message's content."
  (table.concat (icollect [_ block (ipairs (or content []))]
                  (when (and (= block.type :text) (= (type block.text) :string))
                    block.text)) "\n"))

(fn message-effects [message]
  "Present one stored canonical message in the transcript."
  (if (= message.role :user)
      (when (not= (text-of message.content) "")
        [{:type :dispatch
          :event {:type :transcript/user :text (text-of message.content)}}])
      (= message.role :assistant)
      [{:type :dispatch
        :event {:type :transcript/assistant :content message.content}}]
      (= message.role :tool)
      [{:type :dispatch
        :event {:id message.tool_call_id
                :is_error (= message.is_error true)
                :text (text-of message.content)
                :type :transcript/tool-result}}]
      []))

(fn install [db data messages]
  "Adopt a loaded conversation as this session's history."
  (assert (> (length messages) 0) "the stored conversation has no messages")
  (let [fx [{:type :dispatch :event {:type :transcript/reset}}]]
    (each [_ message (ipairs messages)]
      (each [_ effect (ipairs (message-effects message))]
        (table.insert fx effect)))
    (table.insert fx
                  {:type :dispatch
                   :event {:type :agent/conversation-loaded : messages}})
    {:patch {:conversation {:id data.conversation
                            :label (. (or data.metadata {}) :label)
                            :synced (length messages)
                            :pending misa.delete
                            :loading misa.delete}}
     : fx}))

(fn loaded [db event]
  "Accumulate loaded pages, then install the conversation."
  (when event.ok
    (let [data (or event.data {})
          conversation (or db.conversation {})
          entries (or data.entries [])]
      (assert (> (length entries) 0) "a stored conversation page was empty")
      (var messages (icollect [_ message (ipairs (or conversation.loading []))]
                      message))
      (each [_ entry (ipairs entries)]
        (if (= entry.kind :reset) (set messages {})
            (= entry.kind :message) (table.insert messages entry.data)))
      (if (= data.more_entries true)
          {:patch {:conversation {:loading (misa.replace messages)}}
           :fx [{:type :conversation/load
                 :conversation data.conversation
                 :after_seq (. entries (length entries) :seq)
                 :limit 1024
                 :completion :conversation/loaded
                 :id (.. :resume- data.conversation "-"
                         (tostring (. entries (length entries) :seq)))}]}
          (install db data messages)))))

(fn open [db event]
  "Open the conversation picker, or load one named by the command."
  (let [requested (and (= (type event.arguments) :string)
                       (not= event.arguments "") event.arguments)]
    (if requested
        {:fx [{:type :conversation/load
               :conversation requested
               :limit 1024
               :completion :conversation/loaded
               :id (.. :resume- requested)}]}
        {:fx [{:type :conversation/list
               :limit (or (and db.conversation db.conversation.list_limit) 25)
               :completion :conversations/listed
               :id :conversations}]})))

(fn listed [db event]
  "Offer the stored conversations to the picker."
  (when event.ok
    (let [conversations (or (. (or event.data {}) :conversations) [])]
      (if (= (length conversations) 0)
          {:fx [{:type :dispatch
                 :event {:level :error
                         :text "No stored conversations to resume."
                         :type :transcript/harness}}]}
          (let [items (icollect [_ summary (ipairs conversations)]
                        {:description (.. (tostring summary.entry_count)
                                          " entries")
                         :id summary.id
                         :label (or (. (or summary.metadata {}) :label)
                                    summary.id)
                         :search (or (. (or summary.metadata {}) :label)
                                     summary.id)
                         :value summary.id})
                sequence (+ (or (and db.conversation db.conversation.open) 0) 1)
                title "Resume conversation"]
            {:patch {:conversation {:open sequence}}
             :fx [{:type :dispatch
                   :event {:completion :conversation/selected
                           :id :conversations
                           :session (misa.choices.session {: items
                                                           :purpose :conversations
                                                           : title
                                                           :views [:all]}
                                                          db)
                           : title
                           :token (tostring sequence)
                           :type :picker/open}}]})))))

(fn selected [db event]
  "Load the conversation the picker selected."
  (if (or (not= event.picker :conversations)
          (not= event.picker_token
                (tostring (or (and db.conversation db.conversation.open) 0))))
      nil
      (if event.cancelled
          {:fx [{:type :terminal/read}]}
          {:fx [{:type :conversation/load
                 :conversation event.value
                 :limit 1024
                 :completion :conversation/loaded
                 :id (.. :resume- event.value)}
                {:type :terminal/read}]})))

{: start : completed : open : listed : loaded : selected : appended}
