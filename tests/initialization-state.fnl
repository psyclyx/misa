(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:argv [] :config {}})
(local initializers [])
(each [_ name (ipairs [:json :themes :theme/default :components :animations
                       :animation/default :auth :models])]
  (local specs ((. (fennel.dofile (.. :extensions/ name :.fnl)) :setup) context))
  (each [_ spec (ipairs specs.fx)]
    (when (and (= spec.type :register/interceptor)
               (not= spec.value.id :models/input))
      (table.insert initializers spec.value)))
  (misa._setup_effects specs))
(assert (= (length initializers) 2))
(local initial {:db {:unrelated {:value true}} :event {:type :app/start}
                :cofx context :fx []})
(var tx initial)
(each [_ policy (ipairs initializers)]
  (local before (misa.json.encode tx))
  (local result (policy.before tx))
  (assert (= before (misa.json.encode tx)) (.. policy.id " mutated its transaction"))
  (assert (= result.db.unrelated initial.db.unrelated))
  (assert (= result.event initial.event))
  (assert (= result.cofx initial.cofx))
  (assert (= result.fx initial.fx))
  (assert (= (policy.before result) result) (.. policy.id " reinitialized existing state"))
  (local idle (misa.patch tx {:event {:type :unrelated}}))
  (assert (= (policy.before idle) idle))
  (set tx result))
(assert (= initial.db.themes nil))
;; Presentation owners initialize in app/start handlers, covered separately by
;; presentation-startup.fnl. Only auth and models still require this cutover.
(assert (= tx.db.themes nil))
(assert (= tx.db.components nil))
(assert (= tx.db.animations nil))
(assert tx.db.auth_startup.ready)
(assert tx.db.models.entries)
(output "initialization transaction ownership passed\n")
