(local definitions (require :misa.definitions))

;; Submission scheduling is independent of the editor and provider loop. One
;; pending prompt coalesces consecutive submissions, preserving their order.

(fn state [db]
  (or db.queue {:attachments {} :pending "" :sending false}))

(fn append [queue text attachments]
  (assert (= (type text) :string) "queued prompt must be a string")
  (local next {:attachments {} :pending queue.pending :sending queue.sending})
  (when (not= text "")
    (set next.pending (or (and (= queue.pending "") text)
                          (.. queue.pending "\n" text))))
  (each [key image (pairs (or queue.attachments {}))]
    (tset next.attachments key image))
  (each [_ image (ipairs (or attachments {}))]
    (tset next.attachments (+ (length next.attachments) 1) image))
  next)

(fn empty [queue]
  (and (= queue.pending "") (= (length (or queue.attachments {})) 0)))

(fn ready [db queue]
  (and db.agent (= db.agent.status :ready) (not queue.sending)))

(fn drain [db queue]
  (if (or (empty queue) (not (ready db queue))) {:patch {:queue queue}}
      (let [(prompt attachments) (values queue.pending queue.attachments)]
        {:patch {:queue {:pending ""
                         :attachments (misa.replace {})
                         :sending true}}
         :fx [{:event {:attachments attachments
                       :prompt prompt
                       :type :agent/submit}
               :type :dispatch}
              {:type :dispatch :event {:type :queue/submission-settled}}]})))

(fn []
  "Build the declarations for queue."
  (definitions :queue
    [{:catalog :services :id :editor.submit-event :value :queue/submit}
     (let [definition {:id :queue/lifecycle
                       :inputs [[:db/path :queue]]
                       :compute (fn [inputs]
                                  (local queue (. inputs 1))
                                  {:hold_exit (and (not= queue nil)
                                                   (or (not (empty queue))
                                                       (= queue.sending true)))})}]
       {:catalog :subscriptions :id (. definition :id) :value definition})
     {:catalog :services :id :editor.lifecycle.queue :value [:queue/lifecycle]}
     {:catalog :events
      :value {:event :queue/submit
              :handler (fn [db event]
                         (drain db
                                (append (state db) event.prompt
                                        event.attachments)))}}
     {:catalog :events
      :value {:event :agent/status
              :handler (fn [db event]
                         (if (not= event.status :ready)
                             {:patch {:queue (misa.patch (state db)
                                                         {:sending false})}}
                             nil))}}
     {:catalog :events
      :value {:event :agent/completed
              :handler (fn [db]
                         ;; Completion does not acknowledge an outstanding submit.
                         (drain db (state db)))}}
     {:catalog :events
      :value {:event :queue/submission-settled
              :handler (fn []
                         ;; Acknowledge after events emitted by agent/submit, not
                         ;; merely after its state transition. Rejection diagnostics
                         ;; must run before an older completion can permit exit.
                         {:fx [{:type :dispatch
                                :event {:type :queue/submission-acknowledged}}]})}}
     {:catalog :events
      :value {:event :queue/submission-acknowledged
              :handler (fn [db]
                         ;; This continuation runs after agent/submit regardless of
                         ;; handler registration order, including rejected requests.
                         ;; Auth-deferred submissions remain reserved until status changes.
                         (when (not (and db.agent db.agent.startup_prompt))
                           (drain db (misa.patch (state db) {:sending false}))))}}
     {:catalog :events
      :value {:event :agent/reset
              :handler (fn [_]
                         {:patch {:queue (misa.replace {:attachments {}
                                                        :pending ""
                                                        :sending false})}})}}
     {:catalog :events
      :value {:event :queue/take
              :handler (fn [db]
                         (local queue (state db))
                         (if (empty queue) nil
                             (do
                               (local (text attachments)
                                      (values queue.pending queue.attachments))
                               {:patch {:queue {:pending ""
                                                :attachments (misa.replace {})}}
                                :fx [{:event {: attachments
                                              : text
                                              :type :editor/restore}
                                      :type :dispatch}]})))}}
     {:catalog :events
      :value {:event :queue/steer
              :handler (fn [db event]
                         (local queue
                                (append (state db) (or event.prompt "")
                                        event.attachments))
                         (if (empty queue) nil
                             (if (ready db queue) (drain db queue)
                                 {:patch {:queue queue}
                                  :fx [{:event {:type :agent/cancel-active}
                                        :type :dispatch}]})))}}
     (let [definition {:available (fn [db]
                                    (and db.queue (not (empty db.queue))))
                       :event {:type :queue/take}
                       :id :queue.edit
                       :keys [:alt+e]
                       :label "Edit pending message"}]
       {:catalog :actions :id (. definition :id) :value definition})
     (let [definition {:event {:type :editor/steer}
                       :id :queue.steer
                       :keys [:alt+enter]
                       :label "Interrupt and send draft / pending message"}]
       {:catalog :actions :id (. definition :id) :value definition})]
    {}))
