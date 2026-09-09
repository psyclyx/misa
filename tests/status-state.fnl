(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :tests.declarations))
(app.define (. (require :tests.stock) :misa.json))
(local feature (fennel.dofile :extensions/misa/ui/status/init.fnl))
(local specs (. (require :tests.stock) :misa.ui.status))
(local usage-specs (. (require :tests.stock) :misa.usage))
(local handlers {})
(each [_ module (ipairs [specs usage-specs])]
  (each [_ spec (pairs module.events)]
    (when (not (. handlers spec.event)) (tset handlers spec.event []))
    (table.insert (. handlers spec.event) spec.handler)))
(app.define specs)
(app.define usage-specs)
;; Minimal profiles without the indicator registry use the same component model.
(local requests [])
(var observed nil)
(app.define (definitions.collect :fixture [{:catalog :events  :value {:event :app/start :handler (fn [] {:patch {:selected {:provider :kimi :id :first}}})}}
       {:catalog :events  :value {:event :model/select :handler (fn [_ event] {:patch {:selected (misa.replace event.model)}})}}
       {:catalog :events  :value {:event :usage/refresh :handler (fn [_ event] (table.insert requests event.provider) nil)}}
       {:catalog :events  :value {:event :test/read :handler (fn [db] (set observed db) nil)}}]))
(app.install)
(set misa.components {})
(set misa.animations {})
(set misa.models.all {})
(local original-render misa.components.render)
(local original-projection misa.status.indicators)
(local original-animation misa.animations.state)
(local animation-data {:enabled true :frames ["."]})
(set misa.animations.state (fn [_ role] (assert (= role :status)) animation-data))
(set misa.status.indicators nil)
(local fallback-lines [{:spans [{:text :fallback}]}])
(set misa.components.render
     (fn [_ role model context]
       (assert (= role :status.indicators) "status fallback uses a parallel renderer")
       (assert (= (. model.indicators 1 :id) :activity))
       (assert (= (. model.indicators 1 :fact :type) :activity))
       (assert (= (. model.indicators 1 :fact :state) :working))
       (assert (= context.columns 20))
       (assert (= context.activity_animation animation-data))
       {:lines fallback-lines}))
(let [context {:columns 20}]
  (assert (= ((. specs.services :status.model) {:status {:mode :working}} context) fallback-lines))
  (assert (= context.activity_animation nil)))
(set misa.components.render original-render)
(set misa.status.indicators original-projection)
(set misa.animations.state original-animation)
(fn transition [db event]
  (local before (misa.json.encode db))
  (local input (misa.json.encode event))
  (var next-db db)
  (local effects [])
  (each [_ handler (ipairs (or (. handlers event.type) []))]
    (local result (handler next-db event))
    (assert (not (and result result.db)))
    (set next-db (misa.patch next-db (or (and result result.patch) {})))
    (each [_ effect (ipairs (or (and result result.fx) []))]
      (table.insert effects effect)))
  (assert (= before (misa.json.encode db)) "status mutated prior state")
  (assert (= input (misa.json.encode event)) "status mutated event data")
  (values next-db effects))
(local initial (transition {} {:type :app/start}))
(local activity (misa.sub initial [:status/activity]))
(assert (= activity.type :activity))
(assert (= activity.state :ready))
(assert (= activity.spans nil))
(assert (= activity.animation nil))
(assert (= (. (misa.sub initial [:status/session]) :value) 0))
(assert (= (. (misa.sub initial [:status/context]) :type) :tokens)
        "optional models required an absent subscription")
(local context-db (misa.patch initial {:usage {:last_request {:input_tokens 1200 :output_tokens 34}}
                                       :models {:selected :test :entries [{:id :test :context_window 200000}]}}))
(local context-fact (misa.sub context-db [:status/context]))
(assert (= context-fact.type :ratio))
(assert (= context-fact.used 1234))
(assert (= context-fact.limit 200000))
(assert (= context-fact.unit :tokens))
(assert (= context-fact (misa.sub (misa.patch context-db {:hover_action :button}) [:status/context])))
(assert (= activity (misa.sub context-db [:status/activity])))
(assert (= (. (misa.sub (misa.patch context-db {:models {:entries (misa.replace [{:id :test}])}})
                        [:status/context]) :limit) nil) "unknown capacity was invented")
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
                      (each [field key (pairs {:usage :session :last_usage :last_request :provider_usage :providers})]
                        (if (. event field)
                            (assert (= (misa.json.encode (. next.usage key))
                                       (misa.json.encode (. event field))))
                            (assert (= (. next.usage key) (. db.usage key)))))
                      (set db next)))
                  {:cases 500 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(assert (= (transition initial {:type :agent/usage}) initial))
(local quota-db (misa.patch initial {:dialog {:id :usage}
                                   :models {:selected :kimi/model :entries [{:id :kimi/model :provider :kimi}]}
                                   :providers {:kimi {:usage {:windows [{:label "Weekly" :used 25
                                                                        :limit 100 :remaining 75}]}}}}))
;; Registration does not depend on indicator availability, and projection resolves
;; optional services when called, not from a setup-time snapshot.
(local configured (. (require :tests.stock) :misa.ui.status))
(assert (= configured.commands nil))
(assert (= (length (icollect [id (pairs configured.indicators)] id)) 4))
(local projection (. configured.services :status.model))
(set misa.status.indicators nil)
(set misa.components.render nil)
(assert (= (length (projection initial {})) 0))
(set misa.status.indicators (fn [] [:late]))
(assert (= (. (projection initial {}) 1) :late))
(var plan-query nil)
(assert (= configured.indicators.plan.action :usage.open))
(assert (= configured.indicators.plan.value nil))
(set plan-query configured.indicators.plan.query)
(assert plan-query)
(fn plan [db] (misa.sub db plan-query))
(set misa.models.selected (fn [] {:provider :kimi :id :kimi/model}))
(assert (= (plan initial) nil) "missing quota invented a status value")
(assert (= (. (plan quota-db) :type) :percent))
(assert (= (. (plan quota-db) :basis) :remaining))
(assert (= (. (plan quota-db) :value) 75))
(local constrained (misa.patch quota-db
                               {:providers {:kimi {:usage {:windows (misa.replace
                                                                     [{:limit 100 :remaining 75}
                                                                      {:limit 10 :used 9}])}}}}))
(assert (= (. (plan constrained) :value) 10) "widget ignored the tighter quota window")
(local exhausted (misa.patch quota-db {:providers {:kimi {:usage {:windows (misa.replace [{:limit 10 :remaining 0}])}}}}))
(assert (= (. (plan exhausted) :value) 0) "zero remaining quota was treated as absent")
(local unknown (misa.patch quota-db {:providers {:kimi {:usage {:windows (misa.replace [{:used 5}])}}}}))
(assert (= (. (plan unknown) :type) :unavailable) "missing limit became an invented percentage")
(local unavailable (misa.patch quota-db {:providers {:kimi {:usage {:unavailable true}}}}))
(assert (= (. (plan unavailable) :type) :unavailable) "retained windows overrode unavailable")
(assert (= (. (plan unavailable) :reason) :provider_unavailable))
(each [_ window (ipairs [{:used 5 :limit 0} {:used -1 :limit 100} {:used 101 :limit 100}
                         {:remaining math.huge :limit 100} {:remaining 0 :limit math.huge}])]
  (assert (= (. (plan {:models quota-db.models :providers {:kimi {:usage {:windows [window]}}}})
                :type) :unavailable) "invalid quota became a percentage"))
(local quota-fact (plan quota-db))
(assert (= quota-fact (plan (misa.patch quota-db {:hover_action :button}))))
(assert (= quota-fact.value 75))
(set misa.models.selected (fn [] {:provider :other :id :other/model}))
(assert (= (plan (misa.patch quota-db {:models {:selected :other/model
                                               :entries (misa.replace [{:id :other/model :provider :other}])}})) nil)
        "quota widget leaked across selected providers")
(assert (= configured.interceptors nil) "usage refresh must use explicit events")
(set misa.models.selected (fn [db] db.selected))
(local (selected refresh-fx) (transition {:selected {:provider :kimi}} {:type :usage/check-selected}))
(assert (= (. refresh-fx 1 :event :provider) :kimi))
(assert (= (. refresh-fx 1 :event :type) :usage/refresh))
(local (stable stable-fx) (transition selected {:type :usage/check-selected}))
(assert (= stable selected))
(assert (= (length stable-fx) 0))
(assert (= (. handlers :agent/stream-delta) nil) "stream tokens entered quota scheduling")
(assert (= (. handlers :ui/redraw) nil) "redraw entered quota scheduling")
(each [_ name (ipairs [:model/open :model/select :models/provider-availability
                       :models/update :models/replace-provider :auth/ready
                       :transcript/response-end :transcript/response-interrupted])]
  (local (unchanged scheduled) (transition stable {:type name}))
  (assert (= unchanged stable))
  (assert (= (. scheduled 1 :event :type) :usage/check-selected))
  (local (_ completed) (transition stable (. scheduled 1 :event)))
  (local force (or (= name :auth/ready) (= name :transcript/response-end)
                  (= name :transcript/response-interrupted)))
  (assert (= (length completed) (if force 1 0))))
(local (_ switched) (transition (misa.patch stable {:selected {:provider :other}})
                                {:type :usage/check-selected}))
(assert (= (. switched 1 :event :provider) :other))
(local (removed removed-fx) (transition (misa.patch stable {:selected misa.delete})
                                       {:type :usage/check-selected}))
(assert (= removed.usage.quota_provider nil))
(assert (= (length removed-fx) 0))
;; Exercise real dispatch with status registered BEFORE the model owner. Effects
;; run only after commit, so the check must use the newly selected provider.





(fn dispatch [event]
  (local effects (misa._dispatch event {:columns 80 :lines 24 :interactive false}
                                {:wall_ms 0 :monotonic_ms 0}))
  (misa._commit)
  (each [_ effect (ipairs effects)]
    (when (= effect.type :dispatch) (dispatch effect.event))))
(dispatch {:type :app/start})
(assert (= (length requests) 1))
(assert (= (. requests 1) :kimi) "startup check ran before model initialization")
(dispatch {:type :model/select :model {:provider :kimi :id :second}})
(assert (= (length requests) 1) "same-provider selection triggered a refresh")
(dispatch {:type :model/select :model {:provider :other :id :third}})
(assert (= (length requests) 2))
(assert (= (. requests 2) :other) "selection check used the previous provider")
(dispatch {:type :model/select})
(dispatch {:type :test/read})
(assert (= observed.usage.quota_provider nil))
(assert (= (length requests) 2) "provider disappearance scheduled a request")
(dispatch {:type :model/select :model {:provider :kimi :id :first}})
(dispatch {:type :auth/ready})
(dispatch {:type :transcript/response-end})
(dispatch {:type :transcript/response-interrupted})
(assert (= (length requests) 6))
(dispatch {:type :agent/stream-delta})
(dispatch {:type :ui/redraw})
(assert (= (length requests) 6))
(output "status state properties passed\n")
