(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:argv [] :config {}})
(var observed nil)
(var projected nil)
(var derived nil)
(var borrowed nil)
(var borrowed-db nil)
(local before-effect {:type :dispatch :event {:type :test/before}})
(local handler-effect {:type :dispatch :event {:type :test/handler}})
(local app ((require :tests.application) {:argv [] :config {}}))
(local declarations (require :tests.declarations))
(app.define (declarations.collect :state-dispatch-0 [{:catalog :events  :value {:event :test/update :handler (fn [db]
                   (set borrowed-db db)
                   (set borrowed {:patch {:ownership {:value :retained}} :fx [before-effect]})
                   borrowed)}}
       {:catalog :events  :value {:event :test/start :handler (fn [_]
                   {:patch {:value {:a 1 :b 2}}})}}
       {:catalog :events  :value {:event :test/update :handler (fn [_]
                   {:patch {:value {:a 3 :b misa.delete}} :fx [handler-effect]})}}
       {:catalog :events  :value {:event :test/update :handler (fn [db]
                   (assert (= borrowed-db.value.a 1) "dispatch mutated handler input")
                   (assert (= borrowed.patch.ownership.value :retained))
                   (assert (= (length borrowed.fx) 1) "dispatch mutated handler effects")
                   (assert (= (. borrowed.fx 1) before-effect))
                   (assert (= db.value.a 3) "handler did not see preceding patch")
                   (assert (= db.value.b nil) "deletion control reached state")
                   {:patch {:value {:c 4}}})}}
       {:catalog :events  :value {:event :test/fail :handler (fn [_] {:patch {:value {:a 100}}})}}
       {:catalog :events  :value {:event :test/fail :handler (fn [db] {: db :patch {:value {:a 200}}})}}
       {:catalog :events  :value {:event :test/legacy :handler (fn [db] {: db})}}
       {:catalog :events  :value {:event :test/invalid :handler (fn [] {:patch {:bad (fn [])}})}}
       {:catalog :coeffects :id :identity :value (fn [_ _ db] (set derived db) nil)}
       {:catalog :views :id :main :value (fn [db] (set projected db) {:lines []})}
       {:catalog :events  :value {:event :test/read :handler (fn [db] (set observed db) nil)}}]))
(app.install context)
(fn dispatch [event]
  (local effects (misa._dispatch {:type event} {:columns 80 :lines 24 :interactive false}
                                {:wall_ms 0 :monotonic_ms 0}))
  (when (= event :test/update)
    (assert (= (length effects) 2))
    (assert (= (. effects 1 :event :type) :test/before))
    (assert (= (. effects 2 :event :type) :test/handler)))
  (misa._commit)
  (misa._project {:columns 80 :lines 24 :interactive false} {:wall_ms 0 :monotonic_ms 0})
  (misa._commit_projection))
(dispatch :test/start)
(dispatch :test/read)
(local initial observed)
(assert (= initial projected) "view received a state copy")
(assert (= initial derived) "coeffect received a state draft")
(dispatch :test/read)
(assert (= observed initial) "no-op dispatch copied the database")
(dispatch :test/update)
(dispatch :test/read)
(local updated observed)
(assert (not= updated initial))
(assert (= initial.value.a 1) "update mutated retained state")
(assert (= initial.value.b 2) "deletion mutated retained state")
(assert (= updated projected))
(assert (= updated derived))
(assert (not (pcall dispatch :test/fail)) "mixed db/patch result was accepted")
(assert (not (pcall dispatch :test/legacy)) "legacy db result was accepted")
(assert (not (pcall dispatch :test/invalid)) "invalid patch was accepted")
(dispatch :test/read)
(assert (= observed updated) "rejected dispatch replaced committed state")
(assert (= observed.value.a 3) "failed patch transaction committed")
(assert (= observed.value.b nil))
(assert (= observed.value.c 4))
(assert (= borrowed-db initial) "later dispatch changed retained handler input")
(assert (= borrowed.patch.ownership.value :retained) "later dispatch changed retained handler patch")
(assert (= (length borrowed.fx) 1) "later dispatch changed retained handler effects")
(output "patch dispatch contracts passed\n")
