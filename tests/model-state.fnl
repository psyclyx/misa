(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local misa _G.misa)
(local context {:argv [] :config {}})
(app.define ((. (fennel.dofile :extensions/misa/json.fnl) :build) context))
(local handlers {})
(local specs ((. (fennel.dofile :extensions/misa/models/init.fnl) :build) context))
(each [_ spec (pairs (. specs :events))]
  (tset handlers spec.event spec.handler))
(app.define {:subscriptions (. specs :subscriptions)})
(app.install)
(local one {:id :one :provider :a :model :one :context_window 100})
(local two {:id :two :provider :b :model :two :context_window 200})
(local model-db {:models {:entries [one two] :selected :one}})
(local fact (misa.sub model-db [:models/indicator]))
(assert (= fact.type :text))
(assert (= fact.value :a/one))
(assert (= fact (misa.sub (misa.patch model-db {:status {:mode :working}}) [:models/indicator])))
(assert (= (. (misa.sub {} [:models/indicator]) :value) :none))
(local operations
       [{:type :models/provider-availability :provider :a :available false}
        {:type :models/provider-availability :provider :a :available true}
        {:type :models/update :provider :a :models [{:id :one :context_window 500}]}
        {:type :models/update :provider :a :models [{:id :one}]}
        {:type :models/replace-provider :provider :a :authoritative true :models []}
        {:type :models/replace-provider :provider :a :authoritative true :models [one]}])
(local failure
       (G.for_all
         (G.vector (G.elements operations))
         (fn [events]
           (var db {:models {:catalogue [one two] :entries [one two]
                             :available {} :selected :one}})
           (each [_ event (ipairs events)]
             (local before (misa.json.encode db))
             (local result ((. handlers event.type) db event))
             (assert (= before (misa.json.encode db)) "model handler mutated input")
             (set db (misa.patch db result.patch))
             (var found false)
             (each [_ model (ipairs db.models.entries)]
               (assert (not= (. db.models.available model.provider) false)
                       "disabled model remained available")
               (when (= model.id db.models.selected) (set found true)))
             (assert found "selection no longer names an available model")))
         {:cases 1000 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(output "model state properties passed\n")
