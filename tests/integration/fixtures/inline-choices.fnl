(local definitions (require :misa.definitions))

(fn same-data [a b]
  (if (= a b) true
      (or (not= (type a) :table) (not= (type b) :table)) false
      (do
        (var same true)
        (each [key value (pairs a)]
          (when (not (same-data value (. b key))) (set same false)))
        (each [key _ (pairs b)] (when (= (. a key) nil) (set same false)))
        same)))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:description "first positional command"
                                 :event :test/alpha
                                 :name :/alpha}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        (let [definition {:description "nested one"
                                 :event :test/team
                                 :name :/team/one}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        (let [definition {:description "nested two"
                                 :event :test/team
                                 :name :/team/two}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        (let [definition {:completion :test-values
                                 :description "inline overlay"
                                 :event :test/choose
                                 :name :/choose}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        {:catalog :completions :id (.. :test-values "/" (. {:value :alpha} :value)) :value {:group :test-values :value {:value :alpha}}})
          (table.insert declarations
                        {:catalog :completions :id (.. :test-values "/" (. {:label :Beta :value :beta} :value)) :value {:group :test-values :value {:label :Beta :value :beta}}})

          (fn done [text]
            {:fx [{:lines [{:spans [{:style {:foreground :default} : text}]}]
                   :type :view/commit}
                  {:type :app/quit}]})

          (table.insert declarations
                        {:catalog :events  :value {:event :test/alpha :handler (fn [] (done "inline hotkey"))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/choose :handler (fn [_ event]
                                    (assert (= event.arguments :beta))
                                    (done "promoted inline"))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :picker/open :handler (fn [db event]
                                    (if (and (= event.id :inline-choice)
                                             event.choose_view)
                                        (do
                                          (assert (and (= db.picker.id
                                                          :picker-picker)
                                                       (same-data db.picker.parent.session event.session))
                                                  "inline replace_view did not preserve its session in picker-picker")
                                          (done "inline view picker"))
                                        nil))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :terminal/input :handler (fn [db event]
                                    (when (and (= event.kind :text)
                                               (= event.text "/"))
                                      (local projection
                                             (misa.editor.layout db))
                                      (var hinted false)
                                      (each [_ line (ipairs projection.completions)]
                                        (var text "")
                                        (each [_ span (ipairs line.spans)]
                                          (set text (.. text span.text)))
                                        (when (text:find "⌥z" 1 true)
                                          (set hinted true)))
                                      (assert hinted
                                              "inline configured positional hint disappeared"))
                                    nil)}})
          nil
          (definitions.build :tests.integration.fixtures.inline-choices declarations {}))
