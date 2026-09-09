(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local definitions (require :tests.declarations))
(local misa _G.misa)
(local context {:argv [] :config {:animations {:persist false}}})
(local app ((require :tests.application) context))
(app.define (. (require :tests.stock) :misa.json))
(local specs (. (require :tests.stock) :misa.ui.animations))
(local handlers {})
(each [_ spec (pairs (. specs :events))]
  (tset handlers spec.event spec.handler))

(app.define specs)
(app.define (definitions.collect :test
              [{:catalog :animations :id :moving :value {:frames ["a" "b"]}}
               {:catalog :animations :id :still :value {:frames ["a"]}}]))

(app.install)
(local actions [{:type :animations/start :role :status}
                {:type :animations/stop :role :status}
                {:type :animations/tick :id :animation/service}
                {:type :animations/swap :animation :moving}
                {:type :animations/swap :animation :still}])

(local failure (G.for_all (G.vector (G.elements actions))
                          (fn [events]
                            (var db
                                 {:animations {:active :moving
                                               :roles {}
                                               :running {}
                                               :ticks {}}})
                            (each [_ event (ipairs events)]
                              (local before (misa.json.encode db))
                              (local was-running
                                     (= db.animations.timer_running true))
                              (local result
                                     ((. handlers event.type) db event
                                                              {:config context.config}))
                              (assert (= before (misa.json.encode db))
                                      "animation handler mutated input")
                              (when result
                                (set db (misa.patch db result.patch)))
                              (local needed
                                     (and (= db.animations.active :moving)
                                          (= db.animations.running.status true)))
                              (assert (= needed
                                         (= db.animations.timer_running true))
                                      "animation timer state is inconsistent")
                              (var (starts stops) (values 0 0))
                              (each [_ fx (ipairs (or (and result result.fx) []))]
                                (when (= fx.type :timer/start)
                                  (set starts (+ starts 1)))
                                (when (= fx.type :timer/stop)
                                  (set stops (+ stops 1))))
                              (assert (= starts
                                         (if (and needed (not was-running)) 1 0)))
                              (assert (= stops
                                         (if (and was-running (not needed)) 1 0)))))
                          {:cases 1000 :size 20}))

(assert (not failure) (and failure (fennel.view failure)))
(output "animation state properties passed\n")
