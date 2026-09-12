(local definitions (require :tests.declarations))

;; A turn's model call is recorded as the attempt it declares: the row is written
;; before the provider process starts and settled when it exits, so the log holds
;; a model call even though the loop that made it owns no durable state.
(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :agent/completed
                         :handler (fn [db _]
                                    (local conversation
                                           (and db.conversation
                                                db.conversation.id))
                                    (when conversation
                                      {:fx [{:type :conversation/load
                                             :completion :attempt/loaded
                                             : conversation
                                             :id :read-attempts
                                             :limit 8}]}))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :attempt/loaded
                         :handler (fn [_ event]
                                    (assert (= event.ok true)
                                            (.. "the load was refused: "
                                                (tostring event.message)))
                                    (local attempts
                                           (or (. (or event.data {}) :requests)
                                               []))
                                    (local attempt (. attempts 1))
                                    (assert attempt
                                            "the attempt was not recorded")
                                    {:fx [{:lines [{:spans [{:text (.. (tostring attempt.kind)
                                                                       " "
                                                                       (tostring attempt.provider)
                                                                       " "
                                                                       (tostring attempt.model)
                                                                       " "
                                                                       (tostring attempt.status)
                                                                       " "
                                                                       (tostring attempt.cost_kind))}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.attempt-turn declarations {}))
