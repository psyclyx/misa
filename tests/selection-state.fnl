(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(each [_ name (ipairs [:json :layout :markdown :selection_document])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) {}))
(local specs ((. (fennel.dofile :extensions/selection.fnl) :setup)))
(local handlers {})
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler)))
(misa._setup_effects specs)
(local document {:id :doc :kind :document :label :doc :text "one two three"
                 :first 0 :last 13 :children []})
(misa._setup_effects {:fx [{:type :register/selection-source :id :test
                          :value (fn [db] (if db.empty [] [document]))}]})
(fn transition [db type action]
  (local before (misa.json.encode db))
  (local event {: type : action})
  (local result ((. handlers type) db event))
  (assert (= before (misa.json.encode db)) "selection mutated its input")
  (assert (not result.db))
  (values (misa.patch db (or result.patch {})) (or result.fx [])))
(fn opened [empty] (transition {: empty} :selection/open))
(fn act [db action] (transition db :selection/action action))
(local failure
       (G.for_all (G.vector (G.elements [:child :parent :previous :next :first :last
                                        :extend_next :extend_previous :visual :copy :ignore]))
                  (fn [actions]
                    (each [_ empty (ipairs [false true])]
                      (var db (opened empty))
                      (each [_ action (ipairs actions)]
                        (set db (act db action))
                        (local projection (misa.selection_projection db))
                        (when projection
                          (assert (<= 0 projection.first projection.last (length document.text)))))))
                  {:cases 500 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
;; Use explicit structural children to test range anchoring independently of parsing.
(local children [{:kind :word :label :one :first 0 :last 3}
                 {:kind :word :label :two :first 4 :last 7}
                 {:kind :word :label :three :first 8 :last 13}])
(local original-children misa.selection_children)
(set misa.selection_children (fn [] children))
(local leaf (act (opened false) :child))
(local selected (act (act leaf :visual) :next))
(local range (misa.selection_projection selected))
(assert (= range.first 0))
(assert (= range.last 7))
(local (_ effects) (act selected :copy))
(assert (= (. effects 2 :event :text) "one two"))
(assert (= (. (misa.selection_projection (act selected :parent)) :last) 13))
(assert (= (. (misa.selection_projection (act selected :visual)) :first) 4))
(assert (= (. (act selected :close) :selection) nil))
(set misa.selection_children original-children)
(set misa.theme_style (fn [] {:background :red}))
(local lines [{:source_start 0 :source_end 13 :spans [{:text document.text :source true :link :target}]}])
(local before (misa.json.encode lines))
(local decorated (misa.selection_decorate selected :doc document.text lines))
(assert (= before (misa.json.encode lines)) "highlighting mutated render input")
(assert (. decorated 1 :selected))
(assert (= (. decorated 1 :spans 1 :link) :target))
(misa._setup_effects {:fx [{:type :register/selection-action :id :custom
                          :value (fn [state] {:state (misa.patch state {:custom true})})}]})
(assert (. (act selected :custom) :selection :custom))
(output "selection state properties passed\n")
