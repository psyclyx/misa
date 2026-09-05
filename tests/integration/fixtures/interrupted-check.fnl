{:setup (fn []
          (misa.reg_event :agent/completed
                          (fn [db]
                            (assert (= (length db.agent.messages) 1)
                                    "interrupted assistant response entered provider history")
                            (local transcript db.messages.transcript)
                            (assert (and (= (. transcript
                                               (- (length transcript) 1)
                                               :interrupted)
                                            true)
                                         (= (. transcript
                                               (- (length transcript) 1) :text)
                                            :partial))
                                    "partial transcript was not retained")
                            nil))
          nil)}

