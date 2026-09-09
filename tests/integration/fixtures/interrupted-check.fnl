(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :agent/completed :handler (fn [db]
                                    (assert (= (length db.agent.messages) 1)
                                            "interrupted assistant response entered provider history")
                                    (local transcript db.messages.blocks)
                                    (assert (and (= (. transcript
                                                       (- (length transcript) 1)
                                                       :interrupted)
                                                    true)
                                                 (= (. transcript
                                                       (- (length transcript) 1)
                                                       :text)
                                                    :partial))
                                            "partial transcript was not retained")
                                    nil)}})
          nil
          (definitions.build :tests.integration.fixtures.interrupted-check declarations {}))
