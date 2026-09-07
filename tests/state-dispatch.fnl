(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:argv [] :config {}})
(var observed nil)
(misa._setup_effects
 {:fx [{:type :register/event :name :test/start
        :handler (fn [_]
                   {:patch {:value {:a 1 :b 2}}})}
       {:type :register/event :name :test/update
        :handler (fn [_]
                   {:patch {:value {:a 3 :b misa.delete}}})}
       {:type :register/event :name :test/update
        :handler (fn [db]
                   (assert (= db.value.a 3) "handler did not see preceding patch")
                   (assert (= db.value.b nil) "deletion control reached state")
                   {:patch {:value {:c 4}}})}
       {:type :register/event :name :test/fail
        :handler (fn [_] {:patch {:value {:a 100}}})}
       {:type :register/event :name :test/fail
        :handler (fn [db] {: db :patch {:value {:a 200}}})}
       {:type :register/event :name :test/read
        :handler (fn [db] (set observed db) nil)}]})
(misa._seal context)
(fn dispatch [event]
  (misa._dispatch {:type event} {:columns 80 :lines 24 :interactive false}
                  {:wall_ms 0 :monotonic_ms 0})
  (misa._commit))
(dispatch :test/start)
(dispatch :test/update)
(assert (not (pcall dispatch :test/fail)) "mixed db/patch result was accepted")
(dispatch :test/read)
(assert (= observed.value.a 3) "failed patch transaction committed")
(assert (= observed.value.b nil))
(assert (= observed.value.c 4))
(output "patch dispatch contracts passed\n")
