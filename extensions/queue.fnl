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
          ;; Editors choose this capability when installed; alternative submission
          ;; plugins can provide their own event without changing the agent.
          (set misa.submit_event :queue/submit)
          (misa.reg_interceptor {:before (fn [tx]
                                           (local queue tx.db.queue)
                                           (when (and (and (= tx.event.type
                                                              :agent/completed)
                                                           queue)
                                                      (or (not (empty queue))
                                                          queue.sending))
                                             (set tx.event.keep_alive true))
                                           tx)
                                 :id :queue/lifecycle})
          (misa.reg_event :queue/submit
                          (fn [db event]
                            (append (state db) event.prompt event.attachments)
                            (drain db)))
          (misa.reg_event :agent/status
                          (fn [db event]
                            (when (not= event.status :ready)
                              (tset (state db) :sending false))
                            {: db}))
          (misa.reg_event :agent/completed
                          (fn [db] (tset (state db) :sending false) (drain db)))
          (misa.reg_event :agent/reset
                          (fn [db]
                            (set db.queue
                                 {:attachments {} :pending "" :sending false})
                            {: db}))
          (misa.reg_event :queue/take
                          (fn [db]
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
                                         :type :dispatch}]}))))
          (misa.reg_event :queue/steer
                          (fn [db event]
                            (append (state db) (or event.prompt "")
                                    event.attachments)
                            (if (empty (state db)) nil
                                (if (ready db (state db)) (drain db)
                                    {: db
                                     :fx [{:event {:type :agent/cancel-active}
                                           :type :dispatch}]}))))
          (misa.reg_action {:available (fn [db]
                                         (and db.queue (not (empty db.queue))))
                            :event {:type :queue/take}
                            :id :queue.edit
                            :keys [:alt+e]
                            :label "Edit pending message"})
          (misa.reg_action {:event {:type :editor/steer}
                            :id :queue.steer
                            :keys [:alt+enter]
                            :label "Interrupt and send draft / pending message"})
          nil)}

