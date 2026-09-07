(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:config {} :argv []})
(each [_ name (ipairs [:json :layout :choices :values :choice_preview :choice_layout])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))
(local db {})
(local initial (misa.choice_session {:title :Test :views [:all :favorites]
                                    :preference_scope :test
                                    :items [{:id :one :value false}
                                            {:id :two :narrow {:title :Child :items [{:id :child}]}}
                                            {:id :three}]} db))
(fn unchanged [value call]
  (local before (misa.json.encode value))
  (local result (call))
  (assert (= before (misa.json.encode value)) "choice operation mutated its input")
  result)
(local failure
       (G.for_all (G.vector (G.elements [{:action :next} {:action :previous}
                                         {:action :cycle} {:action :cycle_previous}
                                         {:action :accept} {:action :cancel}
                                         {:action :favorite} {:kind :backspace}
                                         {:kind :text :text :t}]))
                  (fn [events]
                    (var session initial)
                    (each [_ event (ipairs events)]
                      (local result (unchanged session #(misa.choice_input session event db)))
                      (set session (assert result.session))
                      (each [_ panel (ipairs session.panels)]
                        (assert (if (= (length panel.items) 0)
                                    (= panel.highlight 0)
                                    (and (>= panel.highlight 1) (<= panel.highlight (length panel.items))))))
                      (local refreshed (unchanged session #(misa.choice_refresh session db)))
                      (assert (= refreshed session) "no-op refresh lost identity")
                      (unchanged session #(misa.choice_rows session db))
                      (unchanged session #(misa.choice_picker_layout session db {:columns 60 :lines 20}))))
                  {:cases 1000 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(local emptied (unchanged initial #(misa.choice_set_items initial [] db)))
(assert (= (length (. emptied.panels 1 :items)) 0))
(assert (= (length (. emptied.view_state :all :items)) 0))
(local child (. (unchanged initial #(misa.choice_accept initial (. initial.items 2) db)) :session))
(local parent (. (unchanged child #(misa.choice_input child {:action :cancel} db)) :session))
(assert (= (fennel.view parent) (fennel.view initial)) "narrowing did not restore its parent")
(unchanged initial #(misa.choice_replace_view initial :favorites db))
(misa._setup_effects {:fx [{:type :register/choice-input :id :clear
                          :value (fn [session] {:session (misa.choice_set_items session [] db) :consumed true})}]})
(assert (= (length (. (misa.choice_input initial {:action :clear} db) :session :items)) 0))
(assert (= (. initial.items 1 :value) false) "false choice value was lost")
(local custom (misa.choice_session {:title :Custom
                                   :view_definitions [{:id :first :items [{:id :one}]}
                                                      {:id :second :items [{:id :two}]}]} db))
(local rotated (. (unchanged custom #(misa.choice_input custom {:action :cycle} db)) :session))
(assert (= (. rotated.panels 1 :id) :second))
(assert (= (. rotated.panels 1 :items 1 :id) :two))
(local handlers {})
(local picker-specs ((. (fennel.dofile :extensions/picker.fnl) :setup)))
(each [_ spec (ipairs picker-specs.fx)]
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler)))
(fn dispatch [previous event]
  (local before (fennel.view event))
  (local result (unchanged previous #((. handlers event.type) previous event
                                     {:terminal {:columns 60 :lines 20}})))
  (assert (= before (fennel.view event)) "picker mutated its event")
  (assert (not result.db))
  (values (misa.patch previous result.patch) result.fx))
(local opened (dispatch {} {:type :picker/open :id :test :token "1"
                           :completion :test/selected :session initial}))
(local (accepted accepted-fx) (dispatch opened {:type :picker/input :action :accept}))
(assert (= accepted.picker nil))
(assert (= (. accepted-fx 1 :event :value) false))
(local replaced (dispatch opened {:type :picker/update :id :test :token "1"
                                 :items [{:id :replacement :value false}] :selected false}))
(assert (= (. replaced.picker.session.items 1 :id) :replacement))
(assert (= replaced.picker.session.selected false))
(local view-picker (dispatch opened {:type :picker/input :action :replace_view}))
(assert (= (fennel.view view-picker.picker.parent) (fennel.view opened.picker)))
(local cancelled (dispatch view-picker {:type :picker/input :action :cancel}))
(assert (= (fennel.view cancelled) (fennel.view opened)))
(local queried (dispatch view-picker {:type :picker/input :kind :text :text :favorites}))
(local changed-view (dispatch queried {:type :picker/input :action :accept}))
(assert (= changed-view.picker.id :test))
(assert (= (. changed-view.picker.session.view_ids 1) :favorites))
(output "choice state properties passed\n")
