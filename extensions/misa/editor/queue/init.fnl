;; Submission scheduling is independent of the editor and provider loop. One
;; pending prompt coalesces consecutive submissions, preserving their order.

(fn state [db]
  "Return the current queue state with initial defaults."
  (or db.queue {:attachments {} :pending "" :sending false}))

(fn append [queue text attachments]
  "Append a prompt and attachments without mutating the pending queue."
  (assert (= (type text) :string) "queued prompt must be a string")
  (let [next {:attachments {} :pending queue.pending :sending queue.sending}]
    (when (not= text "")
      (set next.pending
           (or (and (= queue.pending "") text) (.. queue.pending "\n" text))))
    (each [key image (pairs (or queue.attachments {}))]
      (tset next.attachments key image))
    (each [_ image (ipairs (or attachments {}))]
      (tset next.attachments (+ (length next.attachments) 1) image))
    next))

(fn empty? [queue]
  "Return whether the input queue has no pending entries."
  (and (= queue.pending "") (= (length (or queue.attachments {})) 0)))

(fn ready? [db queue]
  (and db.agent (= db.agent.status :ready) (not queue.sending)))

(fn drain [db queue]
  "Submit pending input when the agent can accept it."
  (if (or (empty? queue) (not (ready? db queue))) {:patch {: queue}}
      (let [prompt queue.pending
            attachments queue.attachments]
        {:patch {:queue {:pending ""
                         :attachments (misa.replace {})
                         :sending true}}
         :fx [{:event {: attachments : prompt :type :agent/submit}
               :type :dispatch}
              {:type :dispatch :event {:type :queue/submission-settled}}]})))

(fn steer [db event]
  "Queue input and request interruption when the agent is busy."
  (let [queue (append (state db) (or event.prompt "") event.attachments)]
    (if (empty? queue) nil
        (if (ready? db queue) (drain db queue)
            {:patch {: queue}
             :fx [{:event {:type :agent/cancel-active} :type :dispatch}]}))))

(fn take [db]
  "Restore pending input to the editor and clear the queue."
  (let [queue (state db)]
    (if (empty? queue) nil
        (let [text queue.pending
              attachments queue.attachments]
          {:patch {:queue {:pending "" :attachments (misa.replace {})}}
           :fx [{:event {: attachments : text :type :editor/restore}
                 :type :dispatch}]}))))

(fn on-agent-reset [_]
  "Clear pending and outstanding input submissions."
  {:patch {:queue (misa.replace {:attachments {} :pending "" :sending false})}})

(fn acknowledge [db]
  "Release a settled submission unless authentication still owns it."
  ;; This continuation runs after agent/submit regardless of
  ;; handler registration order, including rejected requests.
  ;; Auth-deferred submissions remain reserved until status changes.
  (when (not (and db.agent db.agent.startup_prompt))
    (drain db (misa.patch (state db) {:sending false}))))

(fn on-queue-submission-settled []
  "Release the outstanding submission and resume queue delivery."
  ;; Acknowledge after events emitted by agent/submit, not
  ;; merely after its state transition. Rejection diagnostics
  ;; must run before an older completion can permit exit.
  {:fx [{:type :dispatch :event {:type :queue/submission-acknowledged}}]})

(fn on-agent-status [db event]
  "Drain pending input when the agent becomes ready."
  (if (not= event.status :ready)
      {:patch {:queue (misa.patch (state db) {:sending false})}}
      nil))

(fn submit [db event]
  "Queue submitted input and deliver it when the agent is ready."
  (drain db (append (state db) event.prompt event.attachments)))

(fn compute-queue-lifecycle [inputs]
  "Report whether pending submissions should hold editor exit."
  (let [queue (. inputs 1)]
    {:hold_exit (and (not= queue nil)
                     (or (not (empty? queue)) (= queue.sending true)))}))

{: acknowledge
 : append
 : compute-queue-lifecycle
 : drain
 : empty?
 : on-agent-reset
 : on-agent-status
 : on-queue-submission-settled
 : state
 : steer
 : submit
 : take}
