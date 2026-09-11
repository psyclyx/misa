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

(fn message-entries [messages from]
  "Journal entries for canonical messages past FROM."
  (icollect [index message (ipairs messages)]
    (when (> index from) {:kind :message :data message})))

(fn replaced-entries [messages]
  "Journal entries that mark a replaced branch and its new messages."
  (let [result [{:kind :reset :data {:replaced true}}]]
    (each [_ entry (ipairs (message-entries messages 0))]
      (table.insert result entry))
    result))

(fn journal [db]
  "Append canonical history that the durable log has not seen yet."
  (let [conversation (or db.conversation {})
        id conversation.id
        messages (or (and db.agent db.agent.messages) [])
        synced (or conversation.synced 0)]
    (when (and (= (type id) :string) (> (length messages) 0))
      (let [entries (if (< (length messages) synced)
                        (replaced-entries messages)
                        (message-entries messages synced))]
        (when (> (length entries) 0)
          (let [label (or conversation.label (label-of messages))]
            {:patch {:conversation {: label :synced (length messages)}}
             :fx [{:type :conversation/append
                   :conversation id
                   : entries
                   :metadata (when label {: label})
                   :completion :conversation/appended
                   :id (.. id "/" (tostring (length messages)))}]}))))))

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

(fn appended [_ event]
  "Report a failed journal write; a successful one needs no follow-up."
  (when (not event.ok)
    {:fx [{:type :dispatch
           :event {:level :error
                   :text (.. "Could not record the conversation: "
                             (tostring (or event.message :unknown)))
                   :type :transcript/harness}}]}))

{: start : completed : open : listed : loaded : selected : appended}
