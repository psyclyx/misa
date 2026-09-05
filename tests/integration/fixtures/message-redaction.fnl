{:setup (fn []
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:event {:arguments {:token :secret
                                                       :value :abcdef}
                                           :id :call
                                           :name :demo
                                           :type :transcript/tool-call}
                                   :type :dispatch}]}))
          (misa.reg_event :transcript/tool-call
                          (fn [db]
                            (local args
                                   (. db.messages.transcript
                                      (length db.messages.transcript) :arguments))
                            (assert (and (= args.token "[redacted]")
                                         (= args.value
                                            "abc… [truncated 3 bytes]")))
                            {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                     :text :redacted}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

