(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(var (observed computations) (values nil 0))
(var committed-state nil)
(misa._setup_effects
 {:fx [{:type :register/sub
        :value {:id :test :inputs [[:db/path :value] [:db/path :missing]]
                :compute (fn [inputs]
                           (assert (= inputs.n 2))
                           (set computations (+ computations 1))
                           {:value (. inputs 1)})}}
       {:type :register/event :name :set
        :handler (fn [_ event] {:patch {:value event.value :fail (= event.fail true)}})}
       {:type :register/event :name :read
        :handler (fn [db] (set committed-state db) nil)}
       {:type :register/view
        :handler (fn [db]
                   (set observed (misa.sub db [:test]))
                   (assert (not db.fail) "failed speculative view")
                   {:lines []})}]})
(misa._seal {:argv [] :config {}})
(fn dispatch [value fail]
  (misa._dispatch {:type :set : value : fail} {:lines 24 :columns 80}
                  {:wall_ms 0 :monotonic_ms 0})
  (misa._commit))
(fn project []
  (misa._project {:lines 24 :columns 80} {:wall_ms 0 :monotonic_ms 0}))
(dispatch 1)
(project)
(misa._commit_projection)
(local committed observed)
(assert (= computations 1))
(dispatch 2)
(project)
(assert (not= observed committed))
(misa._rollback_projection)
(dispatch 1)
(project)
(assert (= observed committed) "native rejection replaced committed memoization")
(misa._commit_projection)
(dispatch 3 true)
(assert (not (pcall project)))
(misa._rollback_projection)
(misa._dispatch {:type :read} {:lines 24 :columns 80} {:wall_ms 0 :monotonic_ms 0})
(misa._commit)
(assert (= committed-state.value 3) "failed presentation discarded valid model update")
(dispatch 1)
(project)
(assert (= observed committed) "Lua failure replaced committed memoization")
(misa._commit_projection)
(local consumer (misa.subscription_scope 2))
(assert (= (. (consumer.query {:value 5} [:test]) :value) 5))
(assert (<= (consumer.size) 2))
(consumer.close)
(assert (not (pcall consumer.query {} [:test])))
(output "subscription transaction contracts passed\n")
