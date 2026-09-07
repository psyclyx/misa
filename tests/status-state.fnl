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
(assert (: (. effects 1 :event :message) :find "Input tokens: 0" 1 true))
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
  (assert (= indicator-count (if available 3 0)))
  (set misa.indicators_projection nil)
  (set misa.render_component nil)
  (assert (= (length (projection initial {})) 0))
  (set misa.indicators_projection (fn [] [:late]))
  (assert (= (. (projection initial {}) 1) :late)))
(output "status state properties passed\n")
