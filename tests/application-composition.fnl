(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(var observed nil)
(var validated 0)
(local stock
       {:config {:enabled true :nested {:label :stock}}
        :definitions {:services {:fixture (fn [] :stock)
                                 :removed (fn [] :removed)}
                      :examples {:one {:value :original}}
                      :validators {:examples (fn [id value]
                                               (assert (= id :one))
                                               (assert (= value.value :original))
                                               (set validated (+ validated 1)))}
                      :events {:set {:event :change
                                     :priority 1
                                     :handler (fn [] {:patch {:value :old}})}
                               :removed {:event :change
                                         :handler (fn []
                                                    (error "removed handler ran"))}}}})

(local application (misa.snapshot stock))
(set application.config.nested.label :custom)
(set application.definitions.services.fixture (fn [] :replacement))
(set application.definitions.services.removed nil)
(set application.definitions.events.set.handler (fn [] {:patch {:value :new}}))
(set application.definitions.events.removed nil)
(tset application.definitions.events :read
      {:event :change :priority 2 :handler (fn [db] (set observed db.value))})

(assert (= (stock.definitions.services.fixture) :stock)
        "editing a copied application changed stock declarations")

(assert (= stock.config.nested.label :stock))
(assert stock.definitions.services.removed)
(assert (not (pcall misa.configuration))
        "configuration was readable before installation")

(misa._install application.definitions {:argv [] :config application.config})
(assert (= validated 1))
(assert (= (misa.fixture) :replacement))
(assert (= misa.removed nil) "removed service was installed")
(assert (= (. (misa.configuration) :nested :label) :custom))
(set application.config.nested.label :changed-after-install)
(assert (= (. (misa.configuration) :nested :label) :custom)
        "installer retained caller-owned configuration")

(set application.definitions.examples.one.value :mutated)
(assert (= (. (misa.catalog :examples) :one :value) :original)
        "installer retained caller-owned declarations")

(misa._dispatch {:type :change} {:columns 80 :lines 24 :interactive false}
                {:wall_ms 0 :monotonic_ms 0})

(misa._commit)
(assert (= observed :new) "replacement handler did not run before its observer")
(assert (not (pcall misa._install application.definitions {}))
        "application installed twice")

(assert (= misa.compose nil) "obsolete composition API remains exposed")
(output "application data contracts passed\n")
