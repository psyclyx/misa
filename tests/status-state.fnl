(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(misa._setup (fennel.dofile :extensions/json.fnl) {})
(local feature (fennel.dofile :extensions/status.fnl))
(local specs (feature.setup))
(local handlers {})
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler)))
(fn transition [db event]
  (local before (misa.json.encode db))
  (local input (misa.json.encode event))
  (local result ((. handlers event.type) db event))
  (assert (= before (misa.json.encode db)) "status mutated prior state")
  (assert (= input (misa.json.encode event)) "status mutated event data")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(local initial (transition {} {:type :app/start}))
(local failure
       (G.for_all (G.vector (G.elements [{:type :agent/status :status :thinking}
                                         {:type :agent/status :status :ready :usage {}}
                                         {:type :agent/usage :usage {:input_tokens 12 :output_tokens 8}}
                                         {:type :agent/usage :last_usage {:input_tokens 3}}
                                         {:type :agent/usage :provider_usage {:test {:used 10}}}
                                         {:type :agent/usage :provider_usage {}}
                                         {:type :agent/usage}]))
                  (fn [events]
                    (var db initial)
                    (each [_ event (ipairs events)]
                      (local next (transition db event))
                      (each [_ field (ipairs [:usage :last_usage :provider_usage])]
                        (if (. event field)
                            (assert (= (misa.json.encode (. next.status field))
                                       (misa.json.encode (. event field))))
                            (assert (= (. next.status field) (. db.status field)))))
                      (set db next)))
                  {:cases 500 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(assert (= (transition initial {:type :agent/usage}) initial))
(local (_ effects) (transition initial {:type :usage/open}))
(assert (= (. effects 1 :event :type) :dialog/open))
(assert (= (. effects 1 :event :message) nil) "dashboard inserted pre-rendered text")
(assert (= (. effects 1 :event :sections 2 :fields 1 :value) 0))
(assert (= (. effects 2 :event :type) :usage/refresh))
(local quota-db (misa.patch initial {:dialog {:id :usage}
                                   :providers {:kimi {:usage {:windows [{:label "Weekly" :used 25
                                                                        :limit 100 :remaining 75}]}}}}))
(local (_ refreshed) (transition quota-db {:type :usage/updated}))
(assert (= (. refreshed 1 :event :type) :dialog/update))
(assert (= (. refreshed 1 :event :sections 3 :fields 3 :value) 25)
        "quota number was formatted before presentation")
(local (_ closed-refresh) (transition initial {:type :usage/updated}))
(assert (= (length closed-refresh) 0) "usage completion reopened a closed dashboard")
(local provider-db (misa.patch initial {:providers {:z {:subscription_type :Max :usage {:used 1}}
                                                   :a {:usage {:unavailable true}}}
                                       :status {:provider_usage {:z {:used 0}}}}))
(local (_ provider-fx) (transition provider-db {:type :usage/open}))
(local sections (. provider-fx 1 :event :sections))
(assert (= (length sections) 4) "duplicate provider sections")
(assert (= (. sections 3 :title) :a) "provider order is not stable")
(assert (= (. sections 3 :fields 2 :value) "Unavailable"))
(assert (= (. sections 4 :fields 1 :value) :Max))
(assert (= (. sections 4 :fields 2 :value) 0))
(local dialogs ((. (fennel.dofile :extensions/dialogs.fnl) :setup)))
(local dialog-handlers {})
(each [_ spec (ipairs dialogs.fx)]
  (when (= spec.type :register/event) (tset dialog-handlers spec.name spec.handler)))
(local opened ((. dialog-handlers :dialog/open) provider-db (. provider-fx 1 :event)))
(local dialog-db (misa.patch provider-db opened.patch))
(assert (= (misa.json.encode dialog-db.dialog.sections) (misa.json.encode sections)))
(local changed ((. dialog-handlers :dialog/update) dialog-db
                {:id :usage :correlation :usage :sections [{:title :Changed :fields [{:label :Count :value 0}]}]}))
(local next-db (misa.patch dialog-db changed.patch))
(assert (= (. next-db.dialog.sections 1 :title) :Changed))
(assert (= (length dialog-db.dialog.sections) 4))
(misa._setup (fennel.dofile :extensions/layout.fnl) {})
(local component ((. (fennel.dofile :extensions/component/dialog.fnl) :setup)))
(local render (. component.fx 1 :value :render))
(local before-render (misa.json.encode dialog-db.dialog))
(local rendered (render dialog-db.dialog {:columns 80 :available_lines 24}))
(assert (= before-render (misa.json.encode dialog-db.dialog)))
(local text (table.concat (icollect [_ line (ipairs rendered.lines)]
                           (table.concat (icollect [_ span (ipairs line.spans)] span.text))) "\n"))
(assert (text:find "Input tokens: 0" 1 true))
(assert (text:find "Plan usage: Unavailable" 1 true))
(assert (text:find "Plan: Max" 1 true))
;; Registration does not depend on indicator availability, and projection resolves
;; optional services when called, not from a setup-time snapshot.
(each [_ available (ipairs [false true])]
  (set misa.has_setup_effect (fn [] available))
  (local configured (feature.setup))
  (var (command-count projection indicator-count) (values 0 nil 0))
  (each [_ spec (ipairs configured.fx)]
    (when (= spec.type :register/command) (set command-count (+ command-count 1)))
    (when (= spec.type :register/indicator) (set indicator-count (+ indicator-count 1)))
    (when (= spec.name :status_projection) (set projection spec.value)))
  (assert (= command-count 1))
  (assert (= indicator-count (if available 4 0)))
  (set misa.indicators_projection nil)
  (set misa.render_component nil)
  (assert (= (length (projection initial {})) 0))
  (set misa.indicators_projection (fn [] [:late]))
  (assert (= (. (projection initial {}) 1) :late)))
(local configured (feature.setup))
(var plan nil)
(each [_ spec (ipairs configured.fx)]
  (when (and (= spec.type :register/indicator) (= spec.value.id :plan))
    (assert (= spec.value.action :usage.open))
    (set plan spec.value.value)))
(assert plan)
(set misa.selected_model_projection (fn [] {:provider :kimi :id :kimi/model}))
(assert (= (plan initial) nil) "missing quota invented a status value")
(assert (= (plan quota-db) "75% left"))
(local constrained (misa.patch quota-db
                               {:providers {:kimi {:usage {:windows (misa.replace
                                                                     [{:limit 100 :remaining 75}
                                                                      {:limit 10 :used 9}])}}}}))
(assert (= (plan constrained) "10% left") "widget ignored the tighter quota window")
(local exhausted (misa.patch quota-db {:providers {:kimi {:usage {:windows (misa.replace [{:limit 10 :remaining 0}])}}}}))
(assert (= (plan exhausted) "0% left") "zero remaining quota was treated as absent")
(local unknown (misa.patch quota-db {:providers {:kimi {:usage {:windows (misa.replace [{:used 5}])}}}}))
(assert (= (plan unknown) "unavailable") "missing limit became an invented percentage")
(set misa.selected_model_projection (fn [] {:provider :other :id :other/model}))
(assert (= (plan quota-db) nil) "quota widget leaked across selected providers")
(output "status state properties passed\n")
