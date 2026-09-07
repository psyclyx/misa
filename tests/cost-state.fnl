(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local specs ((. (fennel.dofile :extensions/costs.fnl) :setup)
              {:config {:costs {:models {:test {:input 2 :output 4}}}}}))
(var account nil)
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/interceptor) (set account spec.value.before)))
(local transition account)
(set account (fn [tx]
               (local before (fennel.view tx))
               (local result (transition tx))
               (assert (= before (fennel.view tx)) "cost accounting mutated its transaction")
               result))
(local original {:costs {:responses {:earlier {:model :test :cost {:usd 7}}}}})
(local started (account {:db original
                         :event {:type :transcript/response-start
                                 :response_id :current :model :test}}))
(assert (= original.costs.responses.current nil) "start mutated prior state")
(local finished (account {:db started.db
                          :event {:type :transcript/response-end
                                  :response_id :current :usage {:input_tokens 1000000}}}))
(assert (= started.db.costs.responses.current.cost nil) "completion mutated response")
(assert (= finished.db.costs.responses.current.cost.usd 2))
(assert (= finished.db.costs.responses.earlier original.costs.responses.earlier))
(local interrupted (account {:db finished.db
                             :event {:type :transcript/response-interrupted
                                     :response_id :current}}))
(assert (= finished.db.costs.responses.current.cost.usd 2))
(assert interrupted.db.costs.responses.current.cost.unknown)
(local cleared (account {:db interrupted.db :event {:type :transcript/reset}}))
(assert (= (next cleared.db.costs.responses) nil))
(assert interrupted.db.costs.responses.current)
(output "cost state contracts passed\n")
