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
  (and (and db.agent (= db.agent.status :ready)) (not queue.sending)))

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

{:setup (fn []
          {:fx [{:type :register/service
                 :name :submit_event
                 :value :queue/submit}
                {:type :register/sub
                 :value {:id :queue/lifecycle :inputs [[:db/path :queue]]
                         :compute (fn [inputs]
                                    (local queue (. inputs 1))
                                    {:hold_exit (and (not= queue nil)
                                                     (or (not (empty queue)) (= queue.sending true)))})}}
                {:type :register/service :name :editor_lifecycle.queue :value [:queue/lifecycle]}
                {:type :register/event
                 :name :queue/submit
                 :handler (fn [db event]
                            (drain db
                                   (append (state db)
                                           event.prompt
                                           event.attachments)))}
                {:type :register/event
                 :name :agent/status
                 :handler (fn [db event]
                            (if (not= event.status :ready)
                                {:patch {:queue (misa.patch (state db)
                                                           {:sending false})}}
                                nil))}
                {:type :register/event
                 :name :agent/completed
                 :handler (fn [db]
                            ;; Completion does not acknowledge an outstanding submit.
                            (drain db (state db)))}
                {:type :register/event
                 :name :queue/submission-settled
                 :handler (fn []
                            ;; Acknowledge after events emitted by agent/submit, not
                            ;; merely after its state transition. Rejection diagnostics
                            ;; must run before an older completion can permit exit.
                            {:fx [{:type :dispatch :event {:type :queue/submission-acknowledged}}]})}
                {:type :register/event
                 :name :queue/submission-acknowledged
                 :handler (fn [db]
                            ;; This continuation runs after agent/submit regardless of
                            ;; handler registration order, including rejected requests.
                            ;; Auth-deferred submissions remain reserved until status changes.
                            (when (not (and db.agent db.agent.startup_prompt))
                              (drain db (misa.patch (state db) {:sending false}))))}
                {:type :register/event
                 :name :agent/reset
                 :handler (fn [_]
                            {:patch {:queue
                                     (misa.replace {:attachments {}
                                                    :pending ""
                                                    :sending false})}})}
                {:type :register/event
                 :name :queue/take
                 :handler (fn [db]
                            (local queue (state db))
                            (if (empty queue) nil
                                (do
                                  (local (text attachments)
                                         (values queue.pending
                                                 queue.attachments))
                                  {:patch {:queue {:pending ""
                                                   :attachments (misa.replace {})}}
                                   :fx [{:event {: attachments
                                                 : text
                                                 :type :editor/restore}
                                         :type :dispatch}]})))}
                {:type :register/event
                 :name :queue/steer
                 :handler (fn [db event]
                            (local queue
                                   (append (state db)
                                           (or event.prompt "")
                                           event.attachments))
                            (if (empty queue) nil
                                (if (ready db queue) (drain db queue)
                                    {:patch {:queue queue}
                                     :fx [{:event {:type :agent/cancel-active}
                                           :type :dispatch}]})))}
                {:type :register/action
                 :value {:available (fn [db]
                                      (and db.queue (not (empty db.queue))))
                         :event {:type :queue/take}
                         :id :queue.edit
                         :keys [:alt+e]
                         :label "Edit pending message"}}
                {:type :register/action
                 :value {:event {:type :editor/steer}
                         :id :queue.steer
                         :keys [:alt+enter]
                         :label "Interrupt and send draft / pending message"}}]})}
