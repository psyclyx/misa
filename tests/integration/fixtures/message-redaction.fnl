{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:event {:arguments {:token :secret
                                                               :value :abcdef}
                                                   :id :call
                                                   :name :demo
                                                   :type :transcript/tool-call}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/tool-call
                         :handler (fn [db]
                                    (local args
                                           (. db.messages.transcript
                                              (length db.messages.transcript)
                                              :arguments))
                                    (assert (and (= args.token "[redacted]")
                                                 (= args.value
                                                    "abc… [truncated 3 bytes]")))
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :redacted}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
