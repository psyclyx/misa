{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/command
                         :value {:choice_purpose :models
                                 :completion :test-models
                                 :description "choose model"
                                 :event :test/model
                                 :name :/model}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :test-models
                         :value {:value :vendor/one}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :test-models
                         :value {:value :vendor/two}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :terminal/input
                         :handler (fn [db event]
                                    (when (and (and (= event.kind :alt)
                                                    (= event.text :j))
                                               db.editor.choice)
                                      (assert (and (= db.editor.text "/model ")
                                                   (= (length (. db.editor.choice.panels
                                                                 1 :items))
                                                      2))
                                              "command hotkey did not progress into argument completion"))
                                    nil)})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/model
                         :handler (fn [_ event]
                                    (assert (= event.arguments :vendor/one)
                                            "argument hotkey did not execute canonical invocation")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text "narrowed model"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
