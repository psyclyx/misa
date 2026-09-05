;; Submission scheduling is independent of the editor and provider loop. One

;; pending prompt coalesces consecutive submissions, preserving their order.

(fn state [db]
  (set db.queue (or db.queue {:attachments {} :pending "" :sending false}))
  db.queue)

(fn append [queue text attachments]
  (assert (= (type text) :string) "queued prompt must be a string")
  (when (not= text "")
    (set queue.pending (or (and (= queue.pending "") text)
                           (.. queue.pending "\n" text))))
  (set queue.attachments (or queue.attachments {}))
  (each [_ image (ipairs (or attachments {}))]
    (tset queue.attachments (+ (length queue.attachments) 1) image))
  nil)

(fn empty [queue]
  (and (= queue.pending "") (= (length (or queue.attachments {})) 0)))

(fn ready [db queue]
  (and (and db.agent (= db.agent.status :ready)) (not queue.sending)))

(fn drain [db]
  (let [queue (state db)]
    (if (or (empty queue) (not (ready db queue))) {: db}
        (let [(prompt attachments) (values queue.pending queue.attachments)]
          (set (queue.pending queue.attachments queue.sending)
               (values "" {} true))
          {: db
           :fx [{:event {: attachments : prompt :type :agent/submit}
                 :type :dispatch}]}))))

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
                            (append (state db) event.prompt event.attachments)
                            (drain db))}
                {:type :register/event
                 :name :agent/status
                 :handler (fn [db event]
                            (when (not= event.status :ready)
                              (tset (state db) :sending false))
                            {: db})}
                {:type :register/event
                 :name :agent/completed
                 :handler (fn [db] (tset (state db) :sending false)
                            (drain db))}
                {:type :register/event
                 :name :agent/reset
                 :handler (fn [db]
                            (set db.queue
                                 {:attachments {} :pending "" :sending false})
                            {: db})}
                {:type :register/event
                 :name :queue/take
                 :handler (fn [db]
                            (local queue (state db))
                            (if (empty queue) nil
                                (do
                                  (local (text attachments)
                                         (values queue.pending
                                                 queue.attachments))
                                  (set (queue.pending queue.attachments)
                                       (values "" {}))
                                  {: db
                                   :fx [{:event {: attachments
                                                 : text
                                                 :type :editor/restore}
                                         :type :dispatch}]})))}
                {:type :register/event
                 :name :queue/steer
                 :handler (fn [db event]
                            (append (state db) (or event.prompt "")
                                    event.attachments)
                            (if (empty (state db)) nil
                                (if (ready db (state db)) (drain db)
                                    {: db
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
