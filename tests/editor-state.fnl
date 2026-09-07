(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:argv [] :config {:components {:persist false} :themes {:persist false}}})
(each [_ name (ipairs [:json :keybindings :actions :layout :commands :choices
                       :themes :theme/default :components :component/editor
                       :component/picker :choice_layout])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))
(local specs ((. (fennel.dofile :extensions/editor.fnl) :setup) context))
(local handlers {})
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler)))
(misa._setup_effects specs)
(misa._setup_effects {:fx [{:type :register/command :value {:name :/ping :description :Ping :event :test/ping}}
                          {:type :register/command :value {:name :/choose :description :Choose :event :test/choose :completion :test}}
                          {:type :register/completion :group :test :value {:value :alpha}}
                          {:type :register/completion :group :test :value {:value :beta}}]})
(local cofx {:argv [] :terminal {:columns 80 :lines 24 :interactive true}})
(fn transition [db event]
  (local (before input) (values (misa.json.encode db) (misa.json.encode event)))
  (local result ((. handlers event.type) db event cofx))
  (assert (= before (misa.json.encode db)) "editor handler mutated state")
  (assert (= input (misa.json.encode event)) "editor handler mutated event")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (and result result.fx)))
(local initial (transition {:components {:roles {}} :themes {:active :default}} {:type :app/start}))
(local failure
       (G.for_all (G.vector (G.elements [{:type :terminal/input :kind :text :text :a}
                                         {:type :terminal/input :kind :text :text "é"}
                                         {:type :terminal/input :kind :text :text "/"}
                                         {:type :terminal/input :kind :shift_enter}
                                         {:type :terminal/input :kind :backspace}
                                         {:type :terminal/input :kind :arrow_left}
                                         {:type :terminal/input :kind :arrow_right}
                                         {:type :terminal/input :kind :ctrl_c}
                                         {:type :terminal/input :kind :enter}
                                         {:type :editor/attach :attachment {:path :image.png}}
                                         {:type :editor/detach}
                                         {:type :editor/steer}
                                         {:type :editor/restore :text :restored :attachments [{:path :restored.png}]}]))
                  (fn [events]
                    (var db initial)
                    (each [_ event (ipairs events)]
                      (set db (transition db event))
                      (assert (and (>= db.editor.cursor 0) (<= db.editor.cursor (length db.editor.text))))
                      (assert (= db.editor.cursor (misa.layout.boundary_at_or_before db.editor.text db.editor.cursor)))))
                  {:cases 1000 :size 25}))
(assert (not failure) (and failure (fennel.view failure)))
(local attached (transition initial {:type :editor/attach :attachment {:path :old.png}}))
(local restored (transition attached {:type :editor/restore :text :new :attachments [{:path :new.png}]}))
(assert (= (. restored.editor.attachments 1 :path) :new.png))
(assert (= (. restored.editor.attachments 2 :path) :old.png))
(local (submitted submit-fx) (transition restored {:type :terminal/input :kind :enter}))
(assert (= submitted.editor.text ""))
(assert (= (length submitted.editor.attachments) 0))
(assert (= (. submit-fx 1 :event :attachments) restored.editor.attachments))
(assert (= (. submit-fx 1 :event :prompt) :new))
(local chosen (transition initial {:type :terminal/input :kind :text :text "/choose"}))
(local arguments (transition chosen {:type :terminal/input :kind :enter}))
(assert (= arguments.editor.choice_kind :argument))
(assert (= arguments.editor.text "/choose "))
(local picked (transition arguments {:type :terminal/input :kind :text :text :beta}))
(local (executed execute-fx) (transition picked {:type :terminal/input :kind :enter}))
(assert (= (. execute-fx 1 :event :type) :test/choose))
(assert (= (. execute-fx 1 :event :arguments) :beta))
(assert (= executed.editor.text ""))
(local busy (transition restored {:type :agent/status :status :running}))
(local (cancelled cancel-fx) (transition busy {:type :terminal/input :kind :ctrl_c}))
(assert (= cancelled.editor busy.editor))
(assert (= (. cancel-fx 1 :event :type) :agent/cancel-active))
(each [_ cursor (ipairs [-1 1 99])]
  (local bounded (transition initial {:type :editor/restore :replace true :text "é" : cursor}))
  (assert (= bounded.editor.cursor (if (>= cursor 2) 2 0))))
(local before (misa.json.encode picked))
(misa.editor_projection picked {:terminal cofx.terminal})
(assert (= before (misa.json.encode picked)) "editor projection mutated state")
(misa._setup_effects {:fx [{:type :register/editor-edit :id :test_edit
                          :value (fn [editor] (misa.patch editor {:text :custom :cursor 6}))}]})
(assert (= (. (transition initial {:type :terminal/input :kind :test_edit}) :editor :text) :custom))
(output "editor state properties passed\n")
