(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:config {}})
(misa._setup (fennel.dofile :extensions/json.fnl) context)
(misa._setup (fennel.dofile :extensions/preferences.fnl) context)
(local specs ((. (fennel.dofile :extensions/commands.fnl) :setup) context))
(local handlers {})
(each [_ spec (ipairs specs.fx)]
  (assert (not= spec.type :register/interceptor))
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler)))
(misa._setup_effects specs)
(misa._setup_effects
 {:fx [{:type :register/command
        :value {:name :/choose :description "Choose an option" :event :test/choose
                :completion :options :preference_scope :options}}
       {:type :register/command
        :value {:name :/guarded :description "Unavailable choice" :event :test/guarded
                :completion :options :choice_available (fn [db] db.available)
                :choice_unavailable :test/unavailable}}
       {:type :register/completion :group :options :value {:id :first :value :one}}]})
(local db {:preferences {:clock 0 :scopes {}} :unrelated {:value true}})
(fn transition [event]
  (local before (misa.json.encode {: db : event}))
  (local result (handlers.commands/invoke db event))
  (assert (= before (misa.json.encode {: db : event})) "invocation mutated input")
  (local next (misa.patch db (or result.patch {})))
  (assert (= next.unrelated db.unrelated))
  (values result next))
(local event (misa.patch (misa.command_invocation "/choose  one  ")
                         {:correlation {:id :retained}}))
(assert (= event.type :commands/invoke))
(local (normalized next) (transition event))
(local execution (. normalized.fx 2 :event))
(assert (= execution.type :test/choose))
(assert (= execution.arguments :one))
(assert (= execution.canonical "/choose one"))
(assert (= execution.correlation event.correlation))
(assert (= (. next.preferences.scopes.commands "/choose one" :uses) 1))
(assert (= next.preferences.scopes.options.first.uses 1))
(assert (= (length normalized.fx) 2))
(assert (= (. normalized.fx 1 :type) :state/save))
(assert (= (misa.json.encode (. normalized.fx 1 :data))
           (misa.json.encode next.preferences)))
(local (opened unchanged) (transition (misa.command_invocation "/choose")))
(assert (= (. opened.fx 1 :event :type) :choices/command-open))
(assert (= unchanged db))
(local (resumed resumed-db)
       (transition (misa.patch (misa.command_invocation "/choose") {:resumed_choice true})))
(assert (= (. resumed.fx 2 :event :type) :test/choose))
(assert (= (. resumed-db.preferences.scopes.commands :/choose :uses) 1))
(local guarded (handlers.choices/command-open db {:command :/guarded}))
(assert (= (. guarded.fx 1 :event :type) :test/unavailable))
(assert (= guarded.patch nil))
(assert (= (misa.command_invocation "/missing") nil))
(assert (not (pcall handlers.commands/invoke db {:command :/missing})))
(var observed nil)
(var state nil)
(var executions 0)
(misa._setup_effects
 {:fx [{:type :register/event :name :test/choose
        :handler (fn [db event]
                   (set observed event)
                   (set state db)
                   (set executions (+ executions 1))
                   nil)}]})
(misa._seal context)
(fn step [event]
  (local effects (misa._dispatch event {:columns 80 :lines 24 :interactive false}
                                {:wall_ms 0 :monotonic_ms 0}))
  (misa._commit)
  effects)
;; A domain event with command metadata is already an execution, not a request
;; to normalize, count preferences, or open choices again.
(local direct {:type :test/choose :command :/choose :arguments "  direct  "})
(step direct)
(assert (= observed.arguments "  direct  "))
(assert (= state.preferences nil))
(local effects (step (misa.command_invocation "/choose one")))
(assert (= executions 1) "execution bypassed the event queue")
(each [_ effect (ipairs effects)]
  (when (= effect.type :dispatch) (step effect.event)))
(assert (= executions 2))
(assert (= observed.arguments :one))
(assert (= (. state.preferences.scopes.commands "/choose one" :uses) 1))
(assert (= state.preferences.scopes.options.first.uses 1))
(output "explicit command invocation ownership passed\n")
