;; Conversation journal and reopen policy. The durable store itself is covered
;; by the native tests; this checks what the policy writes and reads back.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local conversation (require :misa.conversation))

(local sessions [])
(set _G.misa.choices
     {:session (fn [spec db]
                 (table.insert sessions {: spec : db})
                 {:items spec.items :stub true})})

(fn message [role text]
  {:content [{:type :text : text}] : role})

(fn apply-patch [result]
  "Apply a handler's patch to an empty database."
  (misa.patch {} (. result :patch)))

(fn journal-state [result messages]
  "The database a journal handler patched: a named conversation and history."
  (misa.patch {:conversation {:id :main} :agent {:messages messages}}
              (. result :patch)))

(fn entries-of [result]
  (. result :fx 1 :entries))

(fn start-with [config clock]
  (apply-patch (conversation.start {} {} {: config : clock})))

;; A session names its conversation from configuration or its own clock.
(assert (= (. (start-with {} {:wall_ms 42}) :conversation :id) :session-42))
(assert (= (. (start-with {} {:wall_ms 42}) :conversation :synced) 0))
(assert (= (. (start-with {:conversation {:id :main}} {:wall_ms 42})
              :conversation :id) :main))

(assert (= (conversation.start {} {}
                               {:config {:conversation {:enabled false}}
                                :clock {:wall_ms 42}}) nil)
        "a disabled log still named a conversation")

(assert (not (pcall conversation.start {} {}
                    {:config {:conversation {:id "bad id"}}
                     :clock {:wall_ms 0}}))
        "a conversation id with whitespace was accepted")

(assert (not (pcall conversation.start {} {}
                    {:config {:conversation {:id :a/b}} :clock {:wall_ms 0}}))
        "a conversation id with a slash was accepted")

;; A settled turn journals the messages the log has not seen, and labels the
;; conversation from the first user message.
(local history [(message :user :first) (message :assistant :second)])
(local journaled (conversation.completed {:conversation {:id :main :synced 1}
                                          :agent {:messages history}}
                                         {}))

(assert (= (length (entries-of journaled)) 1) "the wrong tail was journaled")
(assert (= (. (entries-of journaled) 1 :kind) :message))
(assert (= (. (entries-of journaled) 1 :data :role) :assistant))
(assert (= (. journaled :fx 1 :type) :conversation/append))
(assert (= (. journaled :fx 1 :conversation) :main))
(assert (= (. journaled :fx 1 :id) "main/2")
        "the append was not named by the history it covers")
(assert (= (. journaled :fx 1 :metadata :label) :first)
        "the first user message did not label the conversation")
;; The cursor follows the completion, so an unwritten range is not counted as
;; recorded; a failed one is reported and skipped instead of retried forever.
(assert (= (. (apply-patch journaled) :conversation :synced) nil)
        "an unwritten range advanced the cursor")
(assert (= (. (apply-patch journaled) :conversation :pending :kind) :messages))
(assert (= (. (apply-patch journaled) :conversation :pending :last) 2))

(local caught-up (conversation.appended (journal-state journaled history)
                                        {:ok true
                                         :id "main/2"
                                         :data {:count 1 :last_seq 2}}))
(assert (= (. (apply-patch caught-up) :conversation :synced) 2))
(assert (= (. (apply-patch caught-up) :conversation :pending) nil))
(assert (= (. caught-up :fx) nil) "a settled journal started another append")

(local skipped (conversation.appended (journal-state journaled history)
                                      {:ok false
                                       :id "main/2"
                                       :message :EntryTooLarge}))
(assert (= (. (apply-patch skipped) :conversation :synced) 2)
        "an unrecordable range was retried instead of skipped")
(assert (= (. skipped :fx 1 :event :level) :error))
(assert (= (. skipped :fx 1 :event :type) :transcript/harness))
(assert (= (conversation.appended (journal-state caught-up history) {:ok true})
           nil)
        "a settled journal continued after it had caught up")

;; Nothing new, no name, or no history means no write at all.
(assert (= (conversation.completed {:conversation {:id :main :synced 2}
                                    :agent {:messages history}}
                                   {}) nil))

;; An append in flight already covers what a later turn adds.
(assert (= (conversation.completed (apply-patch (conversation.completed
                                                    {:conversation {:id :main
                                                                    :synced 0}
                                                     :agent {:messages history}}
                                                    {}))
                                   {}) nil))

(assert (= (conversation.completed {:agent {:messages history}} {}) nil))
(assert (= (conversation.completed {:conversation {:id :main :synced 0}
                                    :agent {:messages []}}
                                   {}) nil))

;; A turn longer than one append is written in bounded batches, in order.
(fn big-history [count]
  (fcollect [index 1 count] (message :assistant index)))

(local big (big-history 300))
(local long (conversation.completed {:conversation {:id :main :synced 0}
                                     :agent {:messages big}}
                                    {}))
(assert (= (length (. long :fx)) 1) "a long journal was not batched")
(assert (= (length (entries-of long)) 256) "the batch ignored the native limit")
(assert (= (. long :fx 1 :id) "main/256"))
(assert (= (. (apply-patch long) :conversation :pending :last) 256))

(local tail (conversation.appended (journal-state long big)
                                   {:ok true
                                    :id "main/256"
                                    :data {:count 256 :last_seq 256}}))
(assert (= (length (. tail :fx)) 1) "the journal did not continue")
(assert (= (length (entries-of tail)) 44))
(assert (= (. tail :fx 1 :id) "main/300"))
(assert (= (. (apply-patch tail) :conversation :synced) 256)
        "the cursor jumped past the append in flight")
(assert (= (. (apply-patch tail) :conversation :pending :last) 300))

(local end-of-long (conversation.appended (journal-state tail big)
                                          {:ok true
                                           :id "main/300"
                                           :data {:count 44 :last_seq 300}}))
(assert (= (. (apply-patch end-of-long) :conversation :synced) 300))
(assert (= (. end-of-long :fx) nil))

;; A replaced branch is marked first and journaled from the beginning after.
(local replaced (conversation.completed {:conversation {:id :main
                                                        :label :kept
                                                        :synced 5}
                                         :agent {:messages [(message :user
                                                                     :handoff)]}}
                                        {}))

(assert (= (length (. replaced :fx)) 1))
(assert (= (length (entries-of replaced)) 1))
(assert (= (. (entries-of replaced) 1 :kind) :reset))
(assert (= (. (apply-patch replaced) :conversation :synced) 0)
        "a replaced branch kept its old cursor")
(assert (= (. (apply-patch replaced) :conversation :pending :kind) :reset))
(assert (= (. replaced :fx 1 :metadata :label) :kept)
        "a replaced branch forgot its label")

(local rebased (conversation.appended (journal-state replaced
                                                     [(message :user :handoff)])
                                      {:ok true
                                       :id "main/0"
                                       :data {:count 1 :last_seq 6}}))
(assert (= (length (entries-of rebased)) 1))
(assert (= (. (entries-of rebased) 1 :data :role) :user))
(assert (= (. (apply-patch rebased) :conversation :synced) 0))
(assert (= (. (apply-patch rebased) :conversation :pending :last) 1))

;; Installing a stored conversation replays it and adopts its history.
(local stored
       {:ok true
        :data {:conversation :main
               :metadata {:label :stored}
               :more_entries false
               :entries [{:seq 1 :kind :message :data (message :user :hi)}
                         {:seq 2 :kind :message :data (message :assistant :yo)}]}})

(local installed (conversation.loaded {:conversation {:id :session-1 :synced 0}}
                                      stored))

(local adopted (apply-patch installed))
(assert (= (length (. installed :fx)) 4) "the replay emitted the wrong effects")
(assert (= (. installed :fx 1 :event :type) :transcript/reset))
(assert (= (. installed :fx 2 :event :type) :transcript/user))
(assert (= (. installed :fx 2 :event :text) :hi))
(assert (= (. installed :fx 3 :event :type) :transcript/assistant))
(assert (= (. installed :fx 4 :event :type) :agent/conversation-loaded))
(assert (= (length (. installed :fx 4 :event :messages)) 2))
(assert (= (. adopted :conversation :id) :main))
(assert (= (. adopted :conversation :label) :stored))
(assert (= (. adopted :conversation :synced) 2))
(assert (= (. adopted :conversation :loading) nil)
        "installing left a page buffer behind")

;; A longer conversation loads page by page before it is installed.
(local first-page
       {:ok true
        :data {:conversation :main
               :metadata {}
               :more_entries true
               :entries [{:seq 1 :kind :message :data (message :user :a)}]}})

(local pending (conversation.loaded {:conversation {:id :main :synced 0}}
                                    first-page))

(assert (= (length (. pending :fx)) 1) "a partial page was installed early")
(assert (= (. pending :fx 1 :type) :conversation/load))
(assert (= (. pending :fx 1 :after_seq) 1))
(assert (= (length (. (apply-patch pending) :conversation :loading)) 1))
(assert (= (. (apply-patch pending) :conversation :synced) nil))
(local last-page
       {:ok true
        :data {:conversation :main
               :metadata {}
               :more_entries false
               :entries [{:seq 2 :kind :message :data (message :assistant :b)}]}})

(local finished (conversation.loaded {:conversation {:loading [(message :user
                                                                        :a)]}}
                                     last-page))

(assert (= (length (. finished :fx 4 :event :messages)) 2)
        "the earlier page was dropped")

(assert (= (. finished :fx 4 :event :messages 1 :content 1 :text) :a))
(assert (= (. finished :fx 4 :event :messages 2 :content 1 :text) :b))

;; A reset entry starts the history over, discarding earlier pages.
(local reset-page
       {:ok true
        :data {:conversation :main
               :metadata {}
               :more_entries false
               :entries [{:seq 3 :kind :reset :data {:replaced true}}
                         {:seq 4 :kind :message :data (message :user :fresh)}]}})

(local fresh (conversation.loaded {:conversation {:loading [(message :user
                                                                     :stale)]}}
                                  reset-page))

(assert (= (length (. fresh :fx 3 :event :messages)) 1)
        "a replaced branch kept its stale messages")

(assert (= (. fresh :fx 3 :event :messages 1 :content 1 :text) :fresh))

;; A failed load is ignored; an empty page is a contract violation.
(assert (= (conversation.loaded {} {:ok false :message :boom}) nil))
(assert (not (pcall conversation.loaded {}
                    {:ok true :data {:conversation :main :entries []}}))
        "an empty page was accepted")

;; Stored tool messages replay as tool results.
(local tool-page
       {:ok true
        :data {:conversation :main
               :metadata {}
               :more_entries false
               :entries [{:seq 1
                          :kind :message
                          :data {:content [{:text :out :type :text}]
                                 :is_error true
                                 :role :tool
                                 :tool_call_id :call-1}}]}})

(local tool-installed (conversation.loaded {} tool-page))
(assert (= (. tool-installed :fx 2 :event :type) :transcript/tool-result))
(assert (= (. tool-installed :fx 2 :event :id) :call-1))
(assert (= (. tool-installed :fx 2 :event :is_error) true))

;; `/resume` loads the conversation it names, and otherwise lists what exists.
(local list-request (conversation.open {:conversation {:list_limit 7}}
                                       {:arguments ""}))

(assert (= (. list-request :fx 1 :type) :conversation/list))
(assert (= (. list-request :fx 1 :limit) 7))
(local named-request (conversation.open {} {:arguments :abc}))
(assert (= (. named-request :fx 1 :type) :conversation/load))
(assert (= (. named-request :fx 1 :conversation) :abc))

;; The picker receives one labelled item per stored conversation.
(local listed
       (conversation.listed {}
                            {:ok true
                             :data {:conversations [{:entry_count 3
                                                     :id :a
                                                     :metadata {:label :Alpha}}
                                                    {:entry_count 1
                                                     :id :b
                                                     :metadata {}}]}}))

(assert (= (length sessions) 1) "the picker was not opened")
(assert (= (length (. sessions 1 :spec :items)) 2))
(assert (= (. sessions 1 :spec :items 1 :label) :Alpha))
(assert (= (. sessions 1 :spec :items 1 :value) :a))
(assert (= (. sessions 1 :spec :items 1 :description) "3 entries"))
(assert (= (. sessions 1 :spec :items 2 :label) :b)
        "an unlabelled conversation did not fall back to its id")

(assert (= (. (apply-patch listed) :conversation :open) 1))
(assert (= (. listed :fx 1 :event :type) :picker/open))
(assert (= (. listed :fx 1 :event :completion) :conversation/selected))
(assert (= (. listed :fx 1 :event :token) :1))
(let [none (conversation.listed {} {:ok true :data {:conversations []}})]
  (assert (= (. none :fx 1 :event :type) :transcript/harness))
  (assert (= (. none :fx 1 :event :level) :error)))

(assert (= (conversation.listed {} {:ok false :message :no}) nil))

;; A selection is bound to the token the picker was opened with.
(assert (= (conversation.selected {:conversation {:open 2}}
                                  {:picker :conversations
                                   :picker_token :1
                                   :value :a}) nil)
        "a stale picker token was accepted")

(assert (= (conversation.selected {:conversation {:open 2}}
                                  {:picker :other :picker_token :2 :value :a})
           nil))

(let [chosen (conversation.selected {:conversation {:open 2}}
                                    {:picker :conversations
                                     :picker_token :2
                                     :value :a})]
  (assert (= (. chosen :fx 1 :type) :conversation/load))
  (assert (= (. chosen :fx 1 :conversation) :a))
  (assert (= (. chosen :fx 2 :type) :terminal/read)))

(let [cancelled (conversation.selected {:conversation {:open 2}}
                                       {:cancelled true
                                        :picker :conversations
                                        :picker_token :2})]
  (assert (= (. cancelled :fx 1 :type) :terminal/read)))

;; A failed journal write is reported; a settled append with nothing in flight
;; is ignored.
(assert (= (conversation.appended {} {:ok true}) nil))
(assert (= (conversation.appended {:conversation {:synced 2}} {:ok true}) nil))
(let [in-flight {:conversation {:id :main :synced 0
                                :pending {:kind :messages :last 2}}}
      failed (conversation.appended in-flight
                                    {:ok false
                                     :id "main/2"
                                     :message :DatabaseBusy})]
  (assert (= (. (apply-patch failed) :conversation :synced) 2))
  (assert (= (. (apply-patch failed) :conversation :pending) nil))
  (assert (= (. failed :fx 1 :event :type) :transcript/harness))
  (assert (= (. failed :fx 1 :event :level) :error))
  (assert (string.find (. failed :fx 1 :event :text) :DatabaseBusy 1 true))
  (assert (string.find (. failed :fx 1 :event :text) "Could not record" 1 true)))

(output "conversation journal contracts passed\n")
