{:setup (fn []
          (misa.reg_command {:choice_purpose :models
                             :completion :fixture-models
                             :description "Select fixture model"
                             :event :test/model
                             :name :/model
                             :preference_scope :models})
          (misa.reg_completion :fixture-models {:value :vendor/one})
          (misa.reg_completion :fixture-models {:value :vendor/two})
          (misa.reg_event :terminal/input
                          (fn [db]
                            (when (= db.editor.text "/model ")
                              (assert (not db.picker)
                                      "command Enter opened a separate modal picker")
                              (local session (assert db.editor.choice))
                              (assert (and (and (and (= session.title :model)
                                                     (= session.purpose :models))
                                                (= session.input_prefix
                                                   "/model "))
                                           (= session.preference_scope :models))
                                      "argument entry paths disagree")
                              (assert (and (and (and (= session.query "")
                                                     (= (length session.items)
                                                        2))
                                                (= (. session.items 1 :value)
                                                   :vendor/one))
                                           (= (. session.items 1 :invocation)
                                              "/model vendor/one")))
                              (set db.saw_argument_choices true))
                            {: db}))
          (misa.reg_event :test/model
                          (fn [db event]
                            (assert (and db.saw_argument_choices
                                         (= event.arguments :vendor/one))
                                    "choice acceptance did not execute its canonical invocation")
                            {: db
                             :fx [{:lines [{:spans [{:text "command choices"}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

