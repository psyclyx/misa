(local definitions (require :tests.declarations))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:event {:arguments {:token :secret
                                                               :value :abcdef}
                                                   :id :call
                                                   :name :demo
                                                   :type :transcript/tool-call}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :transcript/tool-call :handler (fn [db]
                                    (local args
                                           (. db.messages.blocks
                                              (length db.messages.blocks)
                                              :arguments))
                                    (assert (and (= args.token "[redacted]")
                                                 (= args.value
                                                    "abc… [truncated 3 bytes]")))
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :redacted}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions.collect :tests.integration.fixtures.message-redaction declarations {}))
