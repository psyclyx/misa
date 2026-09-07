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
               :type :dispatch}]})))

{:setup (fn []
          {:fx [{:type :register/service
                 :name :submit_event
                 :value :queue/submit}
                {:type :register/interceptor
                 :value {:before (fn [tx]
                                   (local queue tx.db.queue)
                                   (when (and (and (= tx.event.type
                                                      :agent/completed)
                                                   queue)
                                              (or (not (empty queue))
                                                  queue.sending))
                                     (set tx.event.keep_alive true))
                                   tx)
                         :id :queue/lifecycle}}
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
                            (drain db
                                   (misa.patch (state db)
                                               {:sending false})))}
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
