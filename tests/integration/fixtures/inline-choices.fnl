{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "first positional command"
                                 :event :test/alpha
                                 :name :/alpha}})
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "nested one"
                                 :event :test/team
                                 :name :/team/one}})
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "nested two"
                                 :event :test/team
                                 :name :/team/two}})
          (table.insert setup-fx
                        {:type :register/command
                         :value {:completion :test-values
                                 :description "inline overlay"
                                 :event :test/choose
                                 :name :/choose}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :test-values
                         :value {:value :alpha}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :test-values
                         :value {:label :Beta :value :beta}})

          (fn done [text]
            {:fx [{:lines [{:spans [{:style {:foreground :default} : text}]}]
                   :type :view/commit}
                  {:type :app/quit}]})

          (table.insert setup-fx
                        {:type :register/event
                         :name :test/alpha
                         :handler (fn [] (done "inline hotkey"))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/choose
                         :handler (fn [_ event]
                                    (assert (= event.arguments :beta))
                                    (done "promoted inline"))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :picker/open
                         :handler (fn [db event]
                                    (if (and (= event.id :inline-choice)
                                             event.choose_view)
                                        (do
                                          (assert (and (= db.picker.id
                                                          :picker-picker)
                                                       (= db.picker.parent.session
                                                          event.session))
                                                  "inline replace_view did not preserve its session in picker-picker")
                                          (done "inline view picker"))
                                        nil))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :terminal/input
                         :handler (fn [db event]
                                    (when (and (= event.kind :text)
                                               (= event.text "/"))
                                      (local projection
                                             (misa.editor_projection db))
                                      (var hinted false)
                                      (each [_ line (ipairs projection.completions)]
                                        (var text "")
                                        (each [_ span (ipairs line.spans)]
                                          (set text (.. text span.text)))
                                        (when (text:find "⌥Z" 1 true)
                                          (set hinted true)))
                                      (assert hinted
                                              "inline configured positional hint disappeared"))
                                    nil)})
          nil
          {:fx setup-fx})}
