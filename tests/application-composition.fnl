(local fennel (require :fennel))
(local output io.write)
(local runtime-debug debug)
(local runtime-os os)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(var observed nil)
(var validated 0)
(local first
       {:config {:enabled true :nested {:first true}}
        :definitions {:services {:fixture (fn [] :first)}
                      :examples {:one {:value :original}}
                      :validators {:examples (fn [id value]
                                               (assert (= id :one))
                                               (assert (= value.value :original))
                                               (set validated (+ validated 1)))}
                      :events {:set {:event :change
                                     :handler (fn [] {:patch {:value :old}})}}}})

(local second
       {:config {:nested {:second true}}
        :definitions {:services {:fixture (fn [] :replacement)}
                      :events {:set {:event :change
                                     :priority 1
                                     :handler (fn [] {:patch {:value :new}})}
                               :read {:event :change
                                      :priority 2
                                      :handler (fn [db] (set observed db.value))}}}})

(local application (misa.compose [first second]))
(assert application.config.nested.first)
(assert application.config.nested.second)
(assert (= ((. first.definitions.services :fixture)) :first)
        "composition mutated an input")

(misa._install application.definitions {:argv [] :config application.config})
(assert (= validated 1))
(assert (= (misa.fixture) :replacement))
(set first.definitions.examples.one.value :mutated)
(assert (= (. (misa.catalog :examples) :one :value) :original)
        "installer retained caller-owned definitions")

(misa._dispatch {:type :change} {:columns 80 :lines 24 :interactive false}
                {:wall_ms 0 :monotonic_ms 0})

(misa._commit)
(assert (= observed :new)
        "stable event identity did not replace the earlier handler")

(assert (not (pcall misa._install application.definitions {}))
        "application installed twice")

(local define (require :misa.definitions))
(local handler (fn [] nil))
(local named
       (define.build :fixture
         [{:catalog :events
           :id :explicit
           :value {:event :change :handler handler}}
          {:catalog :events :value {:event :change :handler handler}}
          {:catalog :events :value {:event :change :handler handler}}]))

(assert (. named.events :explicit) "explicit event identity was changed")
(assert (. named.events :fixture/change)
        "unnamed event lacks an owner-local identity")

(assert (. named.events :fixture/change/2)
        "unnamed repeated event lacks a stable suffix")

(assert (not (pcall define :fixture
                    [{:catalog :events
                      :id :duplicate
                      :value {:event :first :handler handler}}
                     {:catalog :events
                      :id :duplicate
                      :value {:event :second :handler handler}}]))
        "duplicate explicit event identities were silently renamed")

(local standard (require :misa.standard))
(fn fresh []
  (set _G.debug runtime-debug)
  (set _G.os runtime-os)
  (fennel.dofile :src/lua_runtime/framework.fnl)
  _G.misa)

(each [_ with-override (ipairs [false true])]
  (local runtime (fresh))
  (local stock
         {:modules {:example {:build (fn [_]
                                       {:services {:example.value (fn [] :stock)}})}}})
  (local override (if with-override
                      {:definitions {:services {:example.value (fn [] :override)}}}
                      {}))
  (local disabled {:definitions {:services {:example.value runtime.delete}}})
  (local spec (runtime.compose [(runtime.compose [stock override]) disabled]))
  (local app (standard.application spec))
  (assert (= (. app.definitions.services :example.value) runtime.delete)
          "module compilation lost an explicit deletion")
  (runtime._install app.definitions {:config {}})
  (assert (= (. (runtime.catalog :services) :example.value) nil)
          "deleted module service was installed")
  (assert (= runtime.example nil) "deleted service created a namespace"))

(let [runtime (fresh)
      base {:definitions {:services {:direct (fn [] :original)}}}
      deleted (runtime.compose [base
                                {:definitions {:services {:direct runtime.delete}}}])]
  (runtime._install deleted.definitions {:config {}})
  (assert (= runtime.direct nil) "direct composition ignored a deletion")
  (assert (= (next (runtime.catalog :services)) nil))
  (assert (= (length (runtime.catalog-entries :services)) 0)))

(let [runtime (fresh)
      restored (runtime.compose [{:definitions {:services {:direct runtime.delete}}}
                                 {:definitions {:services {:direct (fn []
                                                                     :restored)}}}])]
  (runtime._install restored.definitions {:config {}})
  (assert (= (runtime.direct) :restored)
          "later definition did not re-enable its ID"))

(let [runtime (fresh)
      app (runtime.compose [{:definitions {:services {:required (fn []
                                                                  true)}
                                           :requirements {:consumer [:required]}}}
                            {:definitions {:services {:required runtime.delete}}}])
      (ok err) (pcall runtime._install app.definitions {:config {}})]
  (assert (not ok) "deleted dependency unexpectedly satisfied a requirement")
  (assert (string.find (tostring err) "consumer requires required" 1 true)
          "deleted dependency lacks a useful diagnostic"))

(let [runtime (fresh)
      app (standard.application {:modules {:base {:priority 0
                                                  :build (fn [_]
                                                           {:events {:fixture/start {:event :app/start
                                                                                     :handler (fn []
                                                                                                (error "deleted event ran"))}}})}
                                           :disabled {:priority 1
                                                      :build (fn [_]
                                                               {:events {:fixture/start runtime.delete}})}}})]
  (runtime._install app.definitions {:config {}})
  (assert (= (next (runtime.catalog :events)) nil)
          "event priority ordering changed a deletion marker"))

(let [runtime (fresh)
      module-name :fixture.selected-source
      loads {:count 0}]
  (tset package.preload module-name
        (fn []
          (set loads.count (+ loads.count 1))
          (fn [context]
            {:services {:configured (fn [] context.config.value)}})))
  (local spec
         {:config {:value :selected}
          :modules {:selected {:source module-name}
                    :excluded {:source :fixture.must-not-be-loaded}}})
  (local app
         (standard.application (runtime.compose [spec
                                                 {:modules {:excluded runtime.delete}}])))
  (assert (= loads.count 1) "selected source was not loaded once")
  (runtime._install app.definitions {:config app.config})
  (assert (= (runtime.configured) :selected)
          "source constructor lost configuration")
  (assert (= spec.modules.selected.source module-name)
          "construction mutated its specification")
  (assert (not (pcall standard.application
                      {:modules {:invalid {:source module-name
                                           :build (fn [] {})}}}))
          "ambiguous source and build were accepted")
  (tset package.preload module-name nil)
  (tset package.loaded module-name nil))

(output "application composition contracts passed\n")
