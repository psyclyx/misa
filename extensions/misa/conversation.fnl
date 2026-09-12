;; Durable conversation log. Canonical history stays owned by `misa.agent`; this
;; owner writes each message to the log as it becomes final and can put a stored
;; conversation back on request.

;; A conversation's label is merged into the header of every append, so the
;; store's header budget would start refusing writes the moment a configured
;; label exceeded it. The label is validated where it enters instead, against the
;; same bound the native side applies to a request label.
(local max-label-length 256)

(fn label-of-config [value]
  "Read the configured conversation label, or nil when it is absent."
  (when (not= value.label nil)
    (assert (= (type value.label) :string)
            "conversation.label must be a string")
    (assert (and (> (length value.label) 0)
                 (<= (length value.label) max-label-length))
            (.. "conversation.label must be 1 to " (tostring max-label-length)
                " bytes"))
    value.label))

(fn settings [configuration]
  "Read conversation settings with their defaults."
  (let [value (or configuration.conversation {})]
    {:enabled (not= value.enabled false)
     :id (and (= (type value.id) :string) value.id)
     :label (label-of-config value)
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

;; The journal appends one settled message at a time and continues from each
;; completion. Two things follow from that. A turn is durable as it happens
;; instead of at the end, so a crash loses at most the message in flight. And
;; nothing the journal sends can approach what one native append accepts: an
;; append carries a single message, so both the entry count and the batch size
;; stay at one however long a turn runs. A whole message is also the finest unit
;; there is: canonical history only grows in messages — an assistant response is
;; final when its response ends, a tool result when its tool reports.
(fn message-entry [message]
  "One journal entry for a canonical message."
  {:kind :message :data message})

(fn reset-entry []
  "The marker that starts a replaced branch over in the log."
  {:kind :reset :data {:replaced true}})

(fn append-effect [conversation entries label last]
  "One journal append, named by the history it covers."
  {:type :conversation/append
   : conversation
   : entries
   :metadata (when label {: label})
   :completion :conversation/appended
   :id (.. conversation "/" (tostring last))})

(fn next-append [conversation messages synced]
  "The next append of this journal, or nil when it has caught up: the marker
   that starts a replaced branch, or the one message after the cursor. A shorter
   history means the branch was replaced, so its reset marker is sent first and
   the new branch is journaled from the beginning afterwards."
  (let [count (length messages)
        id conversation.id
        label (or conversation.label (label-of messages))]
    (if (< count synced)
        {:patch {:conversation {: label :pending {:kind :reset} :synced 0}}
         :fx [(append-effect id [(reset-entry)] label 0)]}
        (when (< synced count)
          (let [last (+ synced 1)]
            {:patch {:conversation {: label :pending {:kind :messages : last}}}
             :fx [(append-effect id [(message-entry (. messages last))] label
                                 last)]})))))

(fn journal [db _]
  "Record canonical history the log has not seen yet. The agent owner publishes
   a change to canonical history and this runs on it, so a message is written as
   soon as it is final rather than when the turn ends. An append in flight
   already covers what a later change would add, so a change that lands
   meanwhile waits for that chain instead of writing the same range twice."
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
  "Catch the log up once a turn has settled. Every canonical message is written
   as it becomes final, so this closes a gap only if a change reached the agent
   state without publishing one."
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

(fn tool-call-ids [message]
  "The tool call ids of one message, in block order."
  (icollect [_ block (ipairs (or message.content []))]
    (when (= block.type :tool_call) block.id)))

(fn answered-calls [messages from]
  "The tool call ids a result message after FROM answers."
  (let [answered {}]
    (for [index (+ from 1) (length messages)]
      (let [message (. messages index)]
        (when (and (= message.role :tool)
                   (= (type message.tool_call_id) :string))
          (tset answered message.tool_call_id true))))
    answered))

(fn interrupted-results [messages]
  "The tool calls a stored conversation never answered. The log records an
   assistant message as soon as its response ends, so a session can end between
   a tool call and its result. The log keeps saying only what happened — no
   result was recorded — and what that leaves open is closed here."
  (var found nil)
  (var index (length messages))
  (while (and (> index 0) (not found))
    (when (= (. messages index :role) :assistant) (set found index))
    (set index (- index 1)))
  (if (not found)
      []
      (let [answered (answered-calls messages found)
            pending []]
        (each [_ call-id (ipairs (tool-call-ids (. messages found)))]
          (when (and (= (type call-id) :string) (not (. answered call-id)))
            (table.insert pending call-id)))
        pending)))

(fn interrupted-result [call-id]
  "The result a tool call never reported, in the shape a cancelled call takes."
  {:content [{:text "Misa ended before this tool reported a result."
              :type :text}]
   :is_error true
   :role :tool
   :tool_call_id call-id})

(fn closed-history [messages]
  "MESSAGES with every unanswered tool call closed: `(values history closed)`."
  (let [interrupted (interrupted-results messages)
        history (icollect [_ message (ipairs messages)] message)]
    (each [_ call-id (ipairs interrupted)]
      (table.insert history (interrupted-result call-id)))
    (values history (length interrupted))))

(fn install [db data messages]
  "Adopt a loaded conversation as this session's history."
  (assert (> (length messages) 0) "the stored conversation has no messages")
  (let [(history interrupted) (closed-history messages)
        fx [{:type :dispatch :event {:type :transcript/reset}}]]
    (each [_ message (ipairs history)]
      (each [_ effect (ipairs (message-effects message))]
        (table.insert fx effect)))
    (when (> interrupted 0)
      (table.insert fx
                    {:type :dispatch
                     :event {:level :warning
                             :text (.. "The stored conversation ended before "
                                       (tostring interrupted)
                                       (if (= interrupted 1)
                                           " tool result was"
                                           " tool results were")
                                       " recorded; those calls are marked interrupted.")
                             :type :transcript/harness}}))
    (table.insert fx {:type :dispatch
                      :event {:type :agent/conversation-loaded
                              :messages history}})
    {:patch {:conversation {:id data.conversation
                            :label (. (or data.metadata {}) :label)
                            :synced (length history)
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

{: start : journal : completed : open : listed : loaded : selected : appended}
