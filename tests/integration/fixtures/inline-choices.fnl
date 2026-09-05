{:setup (fn []
          (misa.reg_command {:description "first positional command"
                             :event :test/alpha
                             :name :/alpha})
          (misa.reg_command {:description "nested one"
                             :event :test/team
                             :name :/team/one})
          (misa.reg_command {:description "nested two"
                             :event :test/team
                             :name :/team/two})
          (misa.reg_command {:completion :test-values
                             :description "inline overlay"
                             :event :test/choose
                             :name :/choose})
          (misa.reg_completion :test-values {:value :alpha})
          (misa.reg_completion :test-values {:label :Beta :value :beta})

          (fn done [text]
            {:fx [{:lines [{:spans [{:style {:foreground :default} : text}]}]
                   :type :view/commit}
                  {:type :app/quit}]})

          (misa.reg_event :test/alpha (fn [] (done "inline hotkey")))
          (misa.reg_event :test/choose
                          (fn [_ event] (assert (= event.arguments :beta))
                            (done "promoted inline")))
          (misa.reg_event :picker/open
                          (fn [db event]
                            (if (and (= event.id :inline-choice)
                                     event.choose_view)
                                (do
                                  (assert (and (= db.picker.id :picker-picker)
                                               (= db.picker.parent.session
                                                  event.session))
                                          "inline replace_view did not preserve its session in picker-picker")
                                  (done "inline view picker"))
                                nil)))
          (misa.reg_event :terminal/input
                          (fn [db event]
                            (when (and (= event.kind :text) (= event.text "/"))
                              (local projection (misa.editor_projection db))
                              (var hinted false)
                              (each [_ line (ipairs projection.completions)]
                                (var text "")
                                (each [_ span (ipairs line.spans)]
                                  (set text (.. text span.text)))
                                (when (text:find "⌥Z" 1 true)
                                  (set hinted true)))
                              (assert hinted
                                      "inline configured positional hint disappeared"))
                            nil))
          nil)}

