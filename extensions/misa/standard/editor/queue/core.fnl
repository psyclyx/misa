(local {: acknowledge
        : compute-queue-lifecycle
        : drain
        : empty?
        : on-agent-reset
        : on-agent-status
        : on-queue-submission-settled
        : state
        : steer
        : submit
        : take} (require :misa.editor.queue))

{:actions {:queue.edit {:available (fn [db]
                                     (and db.queue (not (empty? db.queue))))
                        :event {:type :queue/take}
                        :id :queue.edit
                        :keys [:alt+e]
                        :label "Edit pending message"}
           :queue.steer {:event {:type :editor/steer}
                         :id :queue.steer
                         :keys [:alt+enter]
                         :label "Interrupt and send draft / pending message"}}
 :events {:queue/queue/submit {:event :queue/submit
                               :handler submit
                               :priority 65000}
          :queue/agent/status {:event :agent/status
                               :handler on-agent-status
                               :priority 65000}
          :queue/agent/completed {:event :agent/completed
                                  :handler (fn [db] (drain db (state db)))
                                  :priority 65000}
          :queue/queue/submission-settled {:event :queue/submission-settled
                                           :handler on-queue-submission-settled
                                           :priority 65000}
          :queue/queue/submission-acknowledged {:event :queue/submission-acknowledged
                                                :handler acknowledge
                                                :priority 65000}
          :queue/agent/reset {:event :agent/reset
                              :handler on-agent-reset
                              :priority 65000}
          :queue/queue/take {:event :queue/take :handler take :priority 65000}
          :queue/queue/steer {:event :queue/steer
                              :handler steer
                              :priority 65000}}
 :services {:editor.submit-event :queue/submit
            :editor.lifecycle.queue [:queue/lifecycle]}
 :subscriptions {:queue/lifecycle {:id :queue/lifecycle
                                   :inputs [[:db/path :queue]]
                                   :compute compute-queue-lifecycle}}}
