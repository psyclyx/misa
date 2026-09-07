{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/command
                         :value {:choice_purpose :models
                                 :completion :fixture-models
                                 :description "Select fixture model"
                                 :event :test/model
                                 :name :/model
                                 :preference_scope :models}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :fixture-models
                         :value {:value :vendor/one}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :fixture-models
                         :value {:value :vendor/two}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :terminal/input
                         :handler (fn [db]
                                    (when (= db.editor.text "/model ")
                                      (assert (not db.picker)
                                              "command Enter opened a separate modal picker")
                                      (local session (assert db.editor.choice))
                                      (assert (and (and (and (= session.title
                                                                :model)
                                                             (= session.purpose
                                                                :models))
                                                        (= session.input_prefix
                                                           "/model "))
                                                   (= session.preference_scope
                                                      :models))
                                              "argument entry paths disagree")
                                      (assert (and (and (and (= session.query
                                                                "")
                                                             (= (length session.items)
                                                                2))
                                                        (= (. session.items 1
                                                              :value)
                                                           :vendor/one))
                                                   (= (. session.items 1
                                                         :invocation)
                                                      "/model vendor/one")))
                                      {:patch {:saw_argument_choices true}}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/model
                         :handler (fn [db event]
                                    (assert (and db.saw_argument_choices
                                                 (= event.arguments :vendor/one))
                                            "choice acceptance did not execute its canonical invocation")
                                    {:fx [{:lines [{:spans [{:text "command choices"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
