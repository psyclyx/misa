(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:argv [] :config {:animations {:persist false}}})
(misa._setup (fennel.dofile :extensions/json.fnl) context)
(local specs ((. (fennel.dofile :extensions/animations.fnl) :setup) context))
(local handlers {})
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler)))
(misa._setup_effects specs)
(misa._setup_effects {:fx [{:type :register/animation :id :moving :value {:frames ["a" "b"]}}
                          {:type :register/animation :id :still :value {:frames ["a"]}}]})
(local actions [{:type :animations/start :role :status}
                {:type :animations/stop :role :status}
                {:type :animations/tick :id :animation/service}
                {:type :animations/swap :animation :moving}
                {:type :animations/swap :animation :still}])
(local failure
       (G.for_all (G.vector (G.elements actions))
                  (fn [events]
                    (var db {:animations {:active :moving :roles {}
                                          :running {} :ticks {}}})
                    (each [_ event (ipairs events)]
                      (local before (misa.json.encode db))
                      (local was-running (= db.animations.timer_running true))
                      (local result ((. handlers event.type) db event))
                      (assert (= before (misa.json.encode db)) "animation handler mutated input")
                      (when result
                        (set db (misa.patch db result.patch)))
                      (local needed (and (= db.animations.active :moving)
                                         (= db.animations.running.status true)))
                      (assert (= needed (= db.animations.timer_running true))
                              "animation timer state is inconsistent")
                      (var (starts stops) (values 0 0))
                      (each [_ fx (ipairs (or (and result result.fx) []))]
                        (when (= fx.type :timer/start) (set starts (+ starts 1)))
                        (when (= fx.type :timer/stop) (set stops (+ stops 1))))
                      (assert (= starts (if (and needed (not was-running)) 1 0)))
                      (assert (= stops (if (and was-running (not needed)) 1 0)))))
                  {:cases 1000 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(output "animation state properties passed\n")
