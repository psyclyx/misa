{:setup (fn []
          (misa.reg_command {:choice_purpose :models
                             :completion :test-models
                             :description "choose model"
                             :event :test/model
                             :name :/model})
          (misa.reg_completion :test-models {:value :vendor/one})
          (misa.reg_completion :test-models {:value :vendor/two})
          (misa.reg_event :terminal/input
                          (fn [db event]
                            (when (and (and (= event.kind :alt)
                                            (= event.text :1))
                                       db.editor.choice)
                              (assert (and (= db.editor.text "/model ")
                                           (= (length (. db.editor.choice.panels
                                                         1 :items))
                                              2))
                                      "command hotkey did not progress into argument completion"))
                            nil))
          (misa.reg_event :test/model
                          (fn [_ event]
                            (assert (= event.arguments :vendor/one)
                                    "argument hotkey did not execute canonical invocation")
                            {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                     :text "narrowed model"}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

