(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:choice_purpose :models
                                 :completion :test-models
                                 :description "choose model"
                                 :event :test/model
                                 :name :/model}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        {:catalog :completions :id (.. :test-models "/" (. {:value :vendor/one} :value)) :value {:group :test-models :value {:value :vendor/one}}})
          (table.insert declarations
                        {:catalog :completions :id (.. :test-models "/" (. {:value :vendor/two} :value)) :value {:group :test-models :value {:value :vendor/two}}})
          (table.insert declarations
                        {:catalog :events  :value {:event :terminal/input :handler (fn [db event]
                                    (when (and (and (= event.kind :alt)
                                                    (= event.text :j))
                                               db.editor.choice)
                                      (assert (and (= db.editor.text "/model ")
                                                   (= (length (. db.editor.choice.panels
                                                                 1 :items))
                                                      2))
                                              "command hotkey did not progress into argument completion"))
                                    nil)}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/model :handler (fn [_ event]
                                    (assert (= event.arguments :vendor/one)
                                            "argument hotkey did not execute canonical invocation")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text "narrowed model"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.command-narrowing declarations {}))
