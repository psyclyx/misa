(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:config {} :argv []})
(each [_ name (ipairs [:json :keybindings :layout :choices :values :choice_preview :choice_layout])]
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

(local branches (misa.choice_session {:title :Branches :views [:all]
                                      :items [{:id "provider/alpha"} {:id "provider/alpine"}]} {}))
(local completed (misa.choice_input branches {:action :complete} {}))
(assert (= completed.session.query "provider/alp"))
(assert (not completed.accepted) "Tab submitted a choice")
(local leaf (misa.choice_input completed.session {:action :complete} {}))
(assert (= leaf.session.query "provider/alpha"))
(assert (not leaf.accepted) "Tab executed the completed leaf")
(local moved (misa.choice_input completed.session {:action :next} {}))
(assert (= (. (misa.choice_input moved.session {:action :complete} {}) :session :query) "provider/alpine"))
(local combo-targets {:option_2_1 (. initial.items 3)})
(local combo (misa.choice_input initial {:kind :alt :text :u :targets combo-targets} {}))
(assert (= combo.session.combo "alt+u"))
(each [_ event (ipairs [{:kind :escape} {:kind :backspace} {:kind :f1} {:kind :arrow_down}])]
  (local aborted (misa.choice_input combo.session event {}))
  (assert (and aborted.consumed (not aborted.session.combo) (not aborted.cancelled)))
  (assert (= aborted.session.query initial.query)))
(local chosen (misa.choice_input combo.session {:kind :text :text :d
                                               :targets {:option_2_1 (. initial.items 3)}} {}))
(assert (= chosen.accepted.id :three))
(assert (not chosen.session.combo))
(assert (= (misa.choice_action {:kind :alt :text "1"}) nil))

(local curated (misa.choice_session {:title :Curated :views [:browse]
                                     :items [{:id :old :browse_visible false} {:id :new}]} {}))
(assert (= (length (. curated.panels 1 :items)) 1))
(local compact-curated (misa.choice_completion_layout curated {} 60 2))
(assert (= (. compact-curated.columns 1 :title) :Suggested))
(assert (= (length (. compact-curated.columns 1 :rows)) 1)
        "a suggestion and its heading must fit in two lines")
(assert (= compact-curated.targets.option_1_1.id :new)
        "visible suggestions must have a working shortcut")
(assert (not (. compact-curated.columns 1 :overflow))
        "a fully visible suggestion must not reserve overflow space")
(each [_ pair (ipairs [[:g :g] [:G :G] [:alt+g "⌥g"]
                       [:ctrl+g "⌃g"] [:ctrl_n "⌃n"] [:f1 :f1]])]
  (assert (= (misa.keybinding_text (. pair 1)) (. pair 2))))
(assert (= (misa.json.encode (misa.render_keybinding :ctrl_n))
           (misa.json.encode (misa.render_keybinding :ctrl+n)))
        "equivalent key encodings must produce identical styled tokens")
(local searched (misa.choice_input curated {:kind :text :text :old} {}))
(assert (= (. searched.session.panels 1 :items 1 :id) :old))
(assert (= (length (. (misa.choice_replace_view curated :all {}) :panels 1 :items)) 2))
(assert (= (. (misa.choice_input initial {:kind :key :key :alt+u :targets {:option_3_1 (. initial.items 3)}} {}) :session :combo) "alt+u"))
(assert (= (misa.choice_action {:kind :key :key "alt+u"}) :sequence))

(local many (misa.choice_session {:title :Many :views [:all]
                                 :items (fcollect [i 1 30] {:id (tostring i)})} {}))
(local many-layout (misa.choice_picker_layout many {} {:columns 100 :lines 40 :available_lines 40}))
;; Test the shared viewport at a height that actually exposes >9 physical rows.
(local many-rows (. (misa.choice_rows many {}) 1 :rows))
(local wide (misa.choice_viewport (. many.panels 1) many-rows 70 30 1 false))
(assert (> (length wide.rows) 9))
(each [_ row (ipairs wide.rows)]
  (assert (and row.hotkey row.action) "visible row has no shortcut")
  (local name (row.action:sub 9))
  (assert (= (. wide.targets name :id) row.id)))
(local overflow (misa.choice_input many {:kind :key :key :alt+o :targets wide.targets} {}))
(assert (= overflow.session.combo "alt+o"))
(local picked (misa.choice_input overflow.session {:kind :text :text :s :targets wide.targets} {}))
(assert (= picked.accepted.id "10"))
(local changed-targets (misa.patch wide.targets {:option_1_10 (misa.replace (. many.items 11))}))
(local stale (misa.choice_input overflow.session {:kind :text :text :s :targets changed-targets} {}))
(assert (and stale.consumed (not stale.accepted) (not stale.session.combo)) "reflow retargeted a pending shortcut")
(local deep-prefix (misa.choice_input many {:kind :alt :text :u :targets wide.targets} {}))
(local continued (misa.choice_input deep-prefix.session {:kind :text :text :r :targets wide.targets} {}))
(assert (= continued.session.combo "alt+u r"))
(local deep (misa.choice_input continued.session {:kind :text :text :l :targets wide.targets} {}))
(assert (= deep.accepted.id "15"))
(local empty-prefix (misa.choice_input many {:kind :alt :text :u :targets {}} {}))
(assert (not empty-prefix.session.combo) "empty panel entered an invisible prefix")
(local two (misa.choice_input initial {:kind :alt :text :u :targets combo-targets} {}))
(local held (misa.choice_input two.session {:kind :alt :text :d :targets combo-targets} {}))
(assert (= held.accepted.id :three))
(local hints {})
(local lengths {})
(local right-keys "jklh;uiomn")
(local left-keys "fdsagrewtq")
(for [bank 1 3]
  (for [slot 1 100]
    (local key (misa.choice_hint (.. "option_" bank "_" slot)))
    (assert key)
    (var count 0)
    (each [letter (: (key:sub 5) :gmatch "%S+")]
      (set count (+ count 1))
      (assert (: (if (= (% count 2) 1) right-keys left-keys) :find letter 1 true)
              "shortcut did not alternate QWERTY hands"))
    (assert (<= count 4) "shortcut grew unnecessarily long")
    (tset lengths count (+ 1 (or (. lengths count) 0)))
    (each [other _ (pairs hints)]
      (assert (and (not= (key:sub 1 (+ (length other) 1)) (.. other " "))
                   (not= (other:sub 1 (+ (length key) 1)) (.. key " "))) "shortcut prefixes collide"))
    (assert (not (. hints key)))
    (tset hints key true)))
(assert (= (. lengths 1) 5))
(assert (= (. lengths 2) 25))
(assert (= (. lengths 3) 125))
(assert (= (misa.choice_hint :option_1_1) "alt+j"))
(assert (= (misa.choice_hint :option_1_6) "alt+u f"))
(assert (= (misa.choice_hint :option_1_8) "alt+i d"))

(local unicode (misa.choice_session {:title :Unicode :views [:all]
                                    :items [{:id "p/α"} {:id "p/β"}]} {}))
(assert (= (. (misa.choice_input unicode {:action :complete} {}) :session :query) "p/"))
(local first-row (. wide.rows 10))
(local resting (misa.choice_row_lines first-row 18))
(local progressing (misa.choice_row_lines (misa.patch first-row {:combo "alt+o"}) 18))
(assert (= (length resting) (length progressing)) "progress changed row height")
(each [index line (ipairs resting)]
  (assert (= (table.concat (icollect [_ span (ipairs line.spans)] span.text))
             (table.concat (icollect [_ span (ipairs (. progressing index :spans))] span.text)))
          "progress changed row text and positional targets"))

(local many-opened (dispatch {} {:type :picker/open :id :many :token :many
                                 :completion :test/selected :session many}))
(local many-pending (dispatch many-opened {:type :picker/input :kind :key :key :alt+o}))
(assert (= many-pending.picker.session.combo "alt+o") "picker adapter lost encoded key")
(local (many-picked many-fx) (dispatch many-pending {:type :picker/input :kind :text :text :s}))
(assert (= (. many-fx 1 :event :value) "10") "visible tenth row shortcut did not select it")
(assert (not many-picked.picker))

(each [_ event (ipairs [{:kind :arrow_down} {:action :next} {:action :complete} {:kind :text :text :x}])]
  (assert (not (misa.choice_needs_targets initial event))))
(each [_ event (ipairs [{:kind :alt :text :o} {:kind :key :key "alt+u"} {:action :option_1_10}])]
  (assert (misa.choice_needs_targets initial event)))
(assert (misa.choice_needs_targets combo.session {:kind :text :text :d}))

;; Built-in projection dependencies must include preferences, while custom views
;; remain free to inspect arbitrary current database fields on every refresh.
(local cache-base (misa.choice_session {:title :Cache :preference_scope :cache
                                       :views [:browse :favorites]
                                       :items [{:id :one} {:id :two}]} {}))
(local first-preferences {:preferences {:scopes {:cache {:one {:favorite true :uses 1 :last 10}}}}})
(local first-cached (misa.choice_refresh cache-base first-preferences))
(assert (= (. first-cached.panels 2 :items 1 :id) :one))
(assert (= (. first-cached.panels 1 :items 1 :section) :Recent))
(assert (= (misa.choice_refresh first-cached (misa.patch first-preferences {:unrelated true})) first-cached)
        "unrelated state lost built-in projection identity")
(local next-preferences (misa.patch first-preferences
                                   {:preferences {:scopes {:cache {:one {:favorite false}
                                                                   :two {:favorite true :uses 2 :last 20}}}}}))
(local next-cached (misa.choice_refresh first-cached next-preferences))
(assert (= (. next-cached.panels 1 :items 1 :id) :two) "Recent did not observe new usage")
(assert (= (length (. next-cached.panels 2 :items)) 1))
(assert (= (. next-cached.panels 2 :items 1 :id) :two) "Favorites cached stale membership")
(assert (= (. (misa.choice_refresh next-cached first-preferences) :panels 2 :items 1 :id) :one)
        "returning to prior preferences reused stale projection")
(misa._setup_effects {:fx [{:type :register/choice-view :id :custom-db-view
                            :value {:project (fn [session _ current]
                                               [(. session.items (if current.pick_second 2 1))])}}]})
(local custom-db (misa.choice_session {:title :Dynamic :views [:custom-db-view]
                                      :items [{:id :one} {:id :two}]} {}))
(assert (= (. custom-db.panels 1 :items 1 :id) :one))
(assert (= (. (misa.choice_refresh custom-db {:pick_second true}) :panels 1 :items 1 :id) :two)
        "custom view lost full-database projection semantics")
