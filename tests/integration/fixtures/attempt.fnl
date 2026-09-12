(local definitions (require :tests.declarations))

;; An attempt is written when the call starts and enriched when it finishes, and
;; a load returns it with the branch it belongs to.
(fn []
  (local declarations [])
  (var settled 0)
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn []
                                    {:fx [{:type :conversation/request
                                           :completion :conversation/requested
                                           :conversation :integration
                                           :id :agent-1
                                           :kind :turn
                                           :model :default
                                           :provider :fake
                                           :status :started}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :conversation/requested
                         :handler (fn [_ event]
                                    (assert (= event.ok true)
                                            (.. "the attempt was refused: "
                                                (tostring event.message)))
                                    (set settled (+ settled 1))
                                    (if (= settled 1)
                                        ;; The outcome write names the same
                                        ;; attempt and adds what only policy knows.
                                        {:fx [{:type :conversation/request
                                               :completion :conversation/requested
                                               :conversation :integration
                                               :cost_kind :reported
                                               :cost_micros 4200
                                               :finished_at_ms 1234
                                               :id :agent-1
                                               :input_tokens 10
                                               :model :default
                                               :output_tokens 4
                                               :provider :fake
                                               :status :ok}]}
                                        {:fx [{:type :conversation/load
                                               :completion :conversation/loaded
                                               :conversation :integration
                                               :id :read-attempts
                                               :limit 8}]}))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :conversation/loaded
                         :handler (fn [_ event]
                                    (assert (= event.ok true)
                                            (.. "the load was refused: "
                                                (tostring event.message)))
                                    (local data (or event.data {}))
                                    (local attempts (or data.requests []))
                                    (local attempt (or (. attempts 1) {}))
                                    {:fx [{:lines [{:spans [{:text (.. (tostring (length attempts))
                                                                       " "
                                                                       (tostring attempt.kind)
                                                                       " "
                                                                       (tostring attempt.status)
                                                                       " "
                                                                       (tostring attempt.cost_micros)
                                                                       " "
                                                                       (tostring attempt.input_tokens)
                                                                       " "
                                                                       (tostring attempt.finished_at_ms))}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.attempt declarations {}))
