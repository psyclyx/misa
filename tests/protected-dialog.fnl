{:setup (fn []
          (misa.reg_fx :input/protected
                       (fn [effect]
                         (assert (and (= effect.id :native)
                                      (= effect.correlation :secret)))
                         {:event {:type :check/protected} :type :dispatch}))
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:event {:actions [{:id :submit
                                                      :label :submit
                                                      :primary true}]
                                           :cancellable true
                                           :completion :check/done
                                           :correlation :secret
                                           :id :native
                                           :initial "must not enter db"
                                           :input true
                                           :protected true
                                           :type :dialog/open}
                                   :type :dispatch}]}))
          (misa.reg_event :check/protected
                          (fn [db]
                            (assert (and (and db.dialog.protected
                                              (= db.dialog.input ""))
                                         (= db.dialog.input_length 0)))
                            {:fx [{:event {:correlation :secret
                                           :id :native
                                           :length 8
                                           :too_long true
                                           :type :dialog/protected-input}
                                   :type :dispatch}
                                  {:event {:type :check/masked}
                                   :type :dispatch}]}))
          (misa.reg_event :check/masked
                          (fn [db]
                            (assert (and (and (= db.dialog.input "")
                                              (= db.dialog.input_length 8))
                                         db.dialog.input_error))
                            {:fx [{:event {:correlation :secret
                                           :id :native
                                           :length 8
                                           :submitted true
                                           :type :dialog/protected-input}
                                   :type :dispatch}]}))
          (misa.reg_event :check/done
                          (fn [db event]
                            (assert (and (and (and event.protected
                                                   (= event.value ""))
                                              (not event.cancelled))
                                         (= db.dialog nil)))
                            {:fx [{:event {:actions [{:id :first :label :first}
                                                     {:id :second
                                                      :label :second
                                                      :primary true}]
                                           :completion :check/action
                                           :correlation :actions
                                           :id :actions
                                           :type :dialog/open}
                                   :type :dispatch}
                                  {:event {:type :check/actions}
                                   :type :dispatch}]}))
          (misa.reg_event :check/actions
                          (fn [db]
                            (assert (= db.dialog.selected_action 2))
                            {:fx [{:event {:kind :tab :type :dialog/input}
                                   :type :dispatch}
                                  {:event {:kind :enter :type :dialog/input}
                                   :type :dispatch}]}))
          (misa.reg_event :check/action
                          (fn [db event]
                            (assert (and (= event.action :first)
                                         (= db.dialog nil)))
                            {:fx [{:lines [{:spans [{:text "protected dialog"}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

