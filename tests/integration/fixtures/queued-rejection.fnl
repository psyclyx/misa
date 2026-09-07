;; Native FIFO: an old completion must not drop a newer rejection's events.
{:setup (fn []
          {:fx [{:type :register/event :name :app/start
                 :handler (fn []
                            {:patch {:agent {:exit_after_response true :startup_prompt misa.delete}
                                     :models (misa.replace {:entries []})}
                             :fx [{:type :dispatch :event {:type :queue/submit :prompt :rejected}}
                                  {:type :dispatch :event {:type :agent/completed :exit true}}]})}
                {:type :register/event :name :transcript/harness
                 :handler (fn [_ event]
                            (assert (= event.problem.code :missing_model))
                            {:patch {:rejection_observed true}})}
                {:type :register/event :name :agent/completed
                 :handler (fn [db]
                            (local count (+ (or db.completions 0) 1))
                            (when (= count 2)
                              (assert db.rejection_observed "rejection completion overtook diagnostic"))
                            {:patch {:completions count}
                             :fx (when (= count 2)
                                   [{:type :view/commit
                                     :lines [{:spans [{:text "rejection completed"}]}]}])})}]})}
