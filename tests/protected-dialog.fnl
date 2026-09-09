{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/fx
                         :name :input/protected
                         :handler (fn [effect]
                                    (assert (and (= effect.id :native)
                                                 (= effect.correlation :secret)))
                                    {:event {:type :check/protected}
                                     :type :dispatch})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
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
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :check/protected
                         :handler (fn [db]
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
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :check/masked
                         :handler (fn [db]
                                    (assert (and (and (= db.dialog.input "")
                                                      (= db.dialog.input_length
                                                         8))
                                                 db.dialog.input_error))
                                    {:fx [{:event {:correlation :secret
                                                   :id :native
                                                   :length 8
                                                   :submitted true
                                                   :type :dialog/protected-input}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :check/done
                         :handler (fn [db event]
                                    (assert (and (and (and event.protected
                                                           (= event.value ""))
                                                      (not event.cancelled))
                                                 (= db.dialog nil)))
                                    {:fx [{:event {:actions [{:id :first
                                                              :label :first}
                                                             {:id :second
                                                              :label :second
                                                              :primary true}]
                                                   :completion :check/action
                                                   :correlation :actions
                                                   :id :actions
                                                   :type :dialog/open}
                                           :type :dispatch}
                                          {:event {:type :check/actions}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :check/actions
                         :handler (fn [db]
                                    ;; Dialog actions use explicit bindings and a primary Enter action;
                                    ;; Tab does not move an implicit button selection.
                                    (assert (. db.dialog.actions 2 :primary))
                                    {:fx [{:event {:kind :tab
                                                   :type :dialog/input}
                                           :type :dispatch}
                                          {:event {:kind :enter
                                                   :type :dialog/input}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :check/action
                         :handler (fn [db event]
                                    (assert (and (= event.action :second)
                                                 (= db.dialog nil)))
                                    {:fx [{:lines [{:spans [{:text "protected dialog"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
