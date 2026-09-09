(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:event {:content [{:text "ordinary **bold**"
                                                              :type :text}
                                                             {:text :thought
                                                              :type :thinking}
                                                             {:arguments {:x 1}
                                                              :id :call
                                                              :name :demo
                                                              :type :tool_call}
                                                             {:text :tail
                                                              :type :text}]
                                                   :request_id :ordered
                                                   :type :transcript/assistant}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :transcript/assistant :handler (fn [db]
                                    (local transcript db.messages.blocks)
                                    (assert (and (and (and (and (= (length transcript)
                                                                   4)
                                                                (= (. transcript
                                                                      1 :kind)
                                                                   :assistant))
                                                           (= (. transcript 2
                                                                 :kind)
                                                              :thinking))
                                                      (= (. transcript 3 :kind)
                                                         :tool_call))
                                                 (= (. transcript 4 :kind)
                                                    :assistant))
                                            "normalized block order changed")
                                    (local before (. transcript 3 :detail))
                                    (local lines
                                           (misa.transcript.project db
                                                                       {:columns 80
                                                                        :interactive true}))
                                    (assert (= (. transcript 3 :detail) before)
                                            "transcript projection mutated its model")
                                    (var bold false)
                                    (each [_ line (ipairs lines)]
                                      (each [_ item (ipairs line.spans)]
                                        (when (and (and (= item.text :bold)
                                                        (= item.style.bold true))
                                                   (= item.style.foreground
                                                      :default))
                                          (set bold true))))
                                    (assert bold
                                            "inline Markdown after ordinary text was not parsed")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :ordered}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.transcript-order declarations {}))
