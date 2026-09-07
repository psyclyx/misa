(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(var (observed computations) (values nil 0))
(misa._setup_effects
 {:fx [{:type :register/sub
        :value {:id :test :inputs [[:db/path :value] [:db/path :missing]]
                :compute (fn [inputs]
                           (assert (= inputs.n 2))
                           (set computations (+ computations 1))
                           {:value (. inputs 1)})}}
       {:type :register/event :name :set
        :handler (fn [_ event] {:patch {:value event.value :fail (= event.fail true)}})}
       {:type :register/view
        :handler (fn [db]
                   (set observed (misa.sub db [:test]))
                   (assert (not db.fail) "failed speculative view")
                   {:lines []})}]})
(misa._seal {:argv [] :config {}})
(fn dispatch [value fail]
  (misa._dispatch {:type :set : value : fail} {:lines 24 :columns 80}
                  {:wall_ms 0 :monotonic_ms 0}))
(dispatch 1)
(misa._commit)
(local committed observed)
(assert (= computations 1))
(dispatch 2)
(assert (not= observed committed))
(misa._rollback)
(dispatch 1)
(assert (= observed committed) "native rejection replaced committed memoization")
(misa._commit)
(assert (not (pcall dispatch 3 true)))
(dispatch 1)
(assert (= observed committed) "Lua failure replaced committed memoization")
(misa._commit)
(local consumer (misa.subscription_scope 2))
(assert (= (. (consumer.query {:value 5} [:test]) :value) 5))
(assert (<= (consumer.size) 2))
(consumer.close)
(assert (not (pcall consumer.query {} [:test])))
(output "subscription transaction contracts passed\n")
