;; Trusted event framework, embedded by Zig.

(local traceback debug.traceback)

(local (events interceptors interceptor-ids) (values {} {} {}))

(local (cofx-fns cofx-order fx-fns) (values {} {} {}))

(local (models model-by-id tools tool-by-name) (values {} {} {} {}))

(local (commands command-by-name) (values {} {}))

(local (actions action-by-id) (values {} {}))

(local (completions completion-values) (values {} {}))

(local (keybindings keybinding-ids) (values {} {}))

(local (auth-providers auth-by-id auth-by-model)
       (values {} {} {}))

(local request-serializers {})

(local (view-layers view-layer-ids) (values {} {}))

;; Subscriptions are pure read models.  A query vector names a registered
;; subscription and carries only serializable arguments.  The registry is
;; deliberately independent of views so other consumers (subagents,
;; background work, tests) can use the same projections.
(local subscription-registry ((require :misa.runtime.subscriptions)))
(var subscription-scope (subscription-registry.scope))
(var active-sub-scope nil)
(var pending-sub-scope nil)

(var (view sealed dispatching db pending-db base-context)
     (values nil false false {} nil nil))

(local MAX_DEPTH 128)

(global misa {:json_null {}})

(local state-updates ((require :misa.runtime.state) misa.json_null))
(set misa.delete state-updates.delete)
(set misa.replace state-updates.replace)
(set misa.patch state-updates.patch)

(local registrations {})

(local setup-handlers {})

(fn runtime-effect [effect]
  (assert (and (and (= (type effect) :table) (= (type effect.type) :string))
               (not= effect.type "")) "effect must have a type")
  (assert (and (not (effect.type:match :^register/))
               (= (. setup-handlers effect.type) nil))
          "setup effects cannot run during event dispatch")
  effect)

(fn open [] (assert (not sealed) "registrations are sealed") nil)

(fn registrations.reg_event [name ___fn___]
  (open)
  (assert (and (= (type name) :string) (not= name "")))
  (assert (= (type ___fn___) :function))
  (local handlers (or (. events name) {}))
  (tset events name handlers)
  (tset handlers (+ (length handlers) 1) ___fn___)
  nil)

(fn registrations.reg_interceptor [value]
  (open)
  (assert (and (and (= (type value) :table) (= (type value.id) :string))
               (not= value.id "")))
  (assert (not (. interceptor-ids value.id)) "duplicate interceptor")
  (assert (or (= value.before nil) (= (type value.before) :function)))
  (assert (or (= value.after nil) (= (type value.after) :function)))
  (tset interceptor-ids value.id true)
  (tset interceptors (+ (length interceptors) 1) value)
  nil)

(fn registrations.reg_cofx [name ___fn___]
  (open)
  (assert (and (= (type name) :string) (not= name "")))
  (assert (and (and (and (not= name :config) (not= name :argv))
                    (not= name :terminal)) (not= name :clock)))
  (assert (and (= (type ___fn___) :function) (= (. cofx-fns name) nil))
          "duplicate cofx")
  (tset cofx-fns name ___fn___)
  (tset cofx-order (+ (length cofx-order) 1) name)
  nil)

(fn registrations.reg_fx [name ___fn___]
  (open)
  (assert (and (= (type name) :string) (not= name "")))
  (runtime-effect {:type name})
  (assert (and (= (type ___fn___) :function) (= (. fx-fns name) nil))
          "duplicate fx")
  (tset fx-fns name ___fn___)
  nil)

(fn registrations.reg_view [___fn___]
  (open)
  (assert (and (= (type ___fn___) :function) (= view nil))
          "view already registered")
  (set view ___fn___)
  nil)

(fn registrations.reg_view_layer [id ___fn___]
  (open)
  (assert (and (= (type id) :string) (not= id ""))
          "view layer ID must be nonempty")
  (assert (and (= (type ___fn___) :function) (not (. view-layer-ids id)))
          "duplicate view layer")
  (tset view-layer-ids id true)
  (tset view-layers (+ (length view-layers) 1) ___fn___)
  nil)

(fn registrations.reg_sub [definition]
  (open)
  (subscription-registry.register definition))

(fn misa.subscription_scope [capacity] (subscription-registry.scope capacity))

(fn misa.sub [state query]
  ((. (or active-sub-scope subscription-scope) :query) state query))

(fn misa.view_layers [state cofx]
  (let [result {}]
    (each [_ project (ipairs view-layers)]
      (local layer (project state cofx))
      (assert (or (= layer nil) (= (type layer) :table))
              "view layer must be a table")
      (when layer
        (tset result (+ (length result) 1) layer)))
    result))

(fn scalar [value]
  (let [kind (type value)]
    (or (or (= kind :string) (= kind :number)) (= kind :boolean))))

(fn registrations.reg_request_options_serializer [id serializer]
  (open)
  (assert (and (= (type id) :string) (not= id ""))
          "request option serializer ID must be nonempty")
  (assert (and (and (= (type serializer) :table)
                    (= (type serializer.accepts) :function))
               (= (type serializer.serialize) :function))
          "invalid request option serializer")
  (assert (= (. request-serializers id) nil)
          (.. "duplicate request option serializer: " id))
  (tset request-serializers id serializer)
  nil)

(fn misa.can_serialize_request_option [id name]
  (let [serializer (. request-serializers id)]
    (and (not= serializer nil) (= (serializer.accepts name) true))))

(fn misa.serialize_request_options [id ___values___ target]
  (let [serializer (assert (. request-serializers id)
                           (.. "unknown request option serializer: "
                               (tostring id)))]
    (assert (= (type ___values___) :table) "request options must be a table")
    (each [name value (pairs ___values___)]
      (assert (= (serializer.accepts name) true)
              (.. "request option is not serializable: " (tostring name)))
      (assert (= (serializer.serialize target name value) true)
              (.. "request option serializer did not consume: " (tostring name))))
    target))

(fn validate-model-api [api]
  (assert (or (= api nil) (= (type api) :table)) "model.api must be a table")
  (if (or (= api nil) (= api.request_options nil)) nil
      (do
        (assert (= (type api.request_options) :table)
                "model.api.request_options must be a table")
        (assert (and (= (type api.request_options_serializer) :string)
                     (not= api.request_options_serializer ""))
                "model request options must identify a serializer")
        (assert (. request-serializers api.request_options_serializer)
                (.. "unknown model request option serializer: "
                    api.request_options_serializer))
        (each [name option (pairs api.request_options)]
          (assert (and (= (type name) :string) (not= name ""))
                  "request option names must be nonempty strings")
          (assert (= (type option) :table)
                  "request option declarations must be tables")
          (assert (or (= option.required nil)
                      (= (type option.required) :boolean))
                  "request option required must be boolean")
          (local choices (or option.choices option.values))
          (assert (or (= choices nil) (= (type choices) :table))
                  "request option choices must be an array")
          (local seen {})
          (each [key (pairs (or choices {}))]
            (assert (and (and (and (= (type key) :number) (>= key 1))
                              (= (% key 1) 0))
                         (<= key (length choices)))
                    "request option choices must be an array"))
          (each [_ choice (ipairs (or choices {}))]
            (assert (scalar choice)
                    "request option choices must be scalar values")
            (local key (.. (type choice) ":" (tostring choice)))
            (assert (not (. seen key)) "request option choices must be unique")
            (tset seen key true))
          (assert (or (= option.default nil) (scalar option.default))
                  "request option default must be a scalar value")
          (when (and (not= option.default nil) choices)
            (local key (.. (type option.default) ":" (tostring option.default)))
            (assert (. seen key)
                    "request option default must be one of its choices")))
        nil)))

(fn registrations.reg_model [model]
  (open)
  (assert (and (and (= (type model) :table) (= (type model.id) :string))
               (not= model.id ""))
          "model.id must be a nonempty string")
  (assert (and (= (type model.provider) :string) (not= model.provider ""))
          "model.provider must be a nonempty string")
  (assert (and (= (type model.model) :string) (not= model.model ""))
          "model.model must be a nonempty string")
  (assert (or (= model.label nil) (= (type model.label) :string))
          "model.label must be a string")
  (assert (or (= model.context_window nil)
              (and (and (= (type model.context_window) :number)
                        (> model.context_window 0))
                   (= (% model.context_window 1) 0)))
          "model.context_window must be a positive integer")
  (validate-model-api model.api)
  (assert (= (. model-by-id model.id) nil) "duplicate model")
  (tset model-by-id model.id model)
  (tset models (+ (length models) 1) model)
  nil)

(fn misa.models [] models)

(fn misa.model [id] (. model-by-id id))

(fn registrations.reg_command [command]
  (open)
  (assert (and (and (and (and (= (type command) :table)
                              (= (type command.name) :string))
                         (command.name:match "^/[%w_/-]+$"))
                    (not (command.name:find "//" 1 true)))
               (not= (command.name:sub (- 1)) "/"))
          "command.name must look like /name or /group/name")
  (assert (= (type command.description) :string)
          "command.description must be a string")
  (assert (and (= (type command.event) :string) (not= command.event ""))
          "command.event must be nonempty")
  (assert (or (= command.completion nil) (= (type command.completion) :string))
          "command.completion must name a completion group")
  (assert (or (= command.complete nil) (= (type command.complete) :function))
          "command.complete must be a function")
  (assert (or (= command.selected nil) (= (type command.selected) :function))
          "command.selected must be a function")
  (assert (or (= command.preference_scope nil)
              (= (type command.preference_scope) :string))
          "command.preference_scope must be a string")
  (assert (or (= command.choice_purpose nil)
              (and (= (type command.choice_purpose) :string)
                   (not= command.choice_purpose "")))
          "command.choice_purpose must be a nonempty string")
  (assert (not (and command.completion command.complete))
          "command may have one completion source")
  (assert (= (. command-by-name command.name) nil) "duplicate command")
  (tset command-by-name command.name command)
  (tset commands (+ (length commands) 1) command)
  nil)

(fn misa.commands [] commands)

(fn misa.command [name] (. command-by-name name))

;; UI actions are discoverable invocations, separate from conversational slash commands.

(fn registrations.reg_action [action]
  (open)
  (assert (and (and (= (type action) :table) (= (type action.id) :string))
               (not= action.id "")) "action ID must be nonempty")
  (assert (and (and (= (type action.label) :string)
                    (= (type action.event) :table))
               (= (type action.event.type) :string))
          "action needs label and event")
  (assert (not (. action-by-id action.id)) "duplicate action")
  (assert (or (= action.available nil) (= (type action.available) :function))
          "action availability must be a function")
  (when (= action.binding nil)
    (set action.binding
         {:action action.id :context (or action.context :global)})
    (registrations.reg_keybinding {:action action.id
                                   :context action.binding.context
                                   :default (or action.keys {})}))
  (assert (and (and (= (type action.binding) :table)
                    (= (type action.binding.context) :string))
               (= (type action.binding.action) :string))
          "action binding must identify context and action")
  (tset action-by-id action.id action)
  (tset actions (+ (length actions) 1) action)
  nil)

(fn misa.actions [] actions)

(fn misa.action [id] (. action-by-id id))

;; Keybinding declarations are data. The keybindings extension owns input

;; normalization and configuration policy; feature extensions only name actions.

(fn registrations.reg_keybinding [binding]
  (open)
  (assert (and (and (= (type binding) :table)
                    (= (type binding.context) :string))
               (not= binding.context ""))
          "keybinding context must be nonempty")
  (assert (and (= (type binding.action) :string) (not= binding.action ""))
          "keybinding action must be nonempty")
  (assert (= (type binding.default) :table)
          "keybinding default must be an array")
  (local id (.. binding.context "/" binding.action))
  (assert (not (. keybinding-ids id)) "duplicate keybinding")
  (each [_ key (ipairs binding.default)]
    (assert (and (= (type key) :string) (not= key ""))
            "keybinding keys must be nonempty strings"))
  (tset keybinding-ids id true)
  (tset keybindings (+ (length keybindings) 1) binding)
  nil)

(fn misa.keybindings [] keybindings)

(fn registrations.reg_completion [group candidate]
  (open)
  (assert (and (= (type group) :string) (not= group ""))
          "completion group must be nonempty")
  (assert (and (and (= (type candidate) :table)
                    (= (type candidate.value) :string))
               (not= candidate.value ""))
          "completion value must be nonempty")
  (assert (or (= candidate.label nil) (= (type candidate.label) :string))
          "completion label must be a string")
  (assert (or (= candidate.description nil)
              (= (type candidate.description) :string))
          "completion description must be a string")
  (tset completions group (or (. completions group) {}))
  (tset completion-values group (or (. completion-values group) {}))
  (assert (not (. completion-values group candidate.value))
          "duplicate completion value")
  (tset (. completion-values group) candidate.value true)
  (tset (. completions group) (+ (length (. completions group)) 1) candidate)
  nil)

(fn registrations.reg_auth_provider [provider]
  (open)
  (assert (and (and (= (type provider) :table) (= (type provider.id) :string))
               (not= provider.id ""))
          "auth provider ID must be nonempty")
  (assert (and (= (type provider.model_provider) :string)
               (not= provider.model_provider ""))
          "auth model provider must be nonempty")
  (assert (or (= provider.discover_models nil)
              (= (type provider.discover_models) :boolean))
          "auth provider discover_models must be boolean")
  (assert (or (or (or (= provider.strategy :api_key)
                      (= provider.strategy :cli_handoff))
                  (= provider.strategy :device_oauth))
              (= provider.strategy :loopback_pkce))
          "auth provider strategy is required")
  (assert (or (= provider.profile nil)
              (and (and (= (type provider.profile) :table)
                        (= (type provider.profile.id) :string))
                   (not= provider.profile.id "")))
          "invalid auth provider profile")
  (when (= provider.strategy :device_oauth)
    (assert (and (and (= (type provider.profile) :table)
                      (= (type provider.profile.authorization_url) :string))
                 (= (type provider.profile.token_url) :string))
            "device OAuth requires endpoint profile"))
  (assert (not (. auth-by-id provider.id)) "duplicate auth provider")
  (assert (not (. auth-by-model provider.model_provider))
          "duplicate auth model provider")
  (tset auth-by-id provider.id provider)
  (tset auth-by-model provider.model_provider provider)
  (tset auth-providers (+ (length auth-providers) 1) provider)
  (registrations.reg_completion :auth-provider
                                {:description provider.description
                                 :label provider.label
                                 :value provider.id})
  nil)

(fn misa.auth_providers [] auth-providers)
(fn misa.auth_provider [id] (. auth-by-id id))
(fn misa.auth_provider_for_model [id] (. auth-by-model id))

(fn misa.command_completions [command prefix state]
  (assert (and (= (type command) :table) (= (type prefix) :string))
          "invalid completion request")
  (local source (or (or (and command.complete (command.complete prefix state))
                        (. completions command.completion))
                    {}))
  (assert (= (type source) :table) "command completer must return an array")
  (each [_ candidate (ipairs source)]
    (assert (and (= (type candidate) :table) (= (type candidate.value) :string))
            "invalid completion candidate"))
  (if misa.fuzzy_choices (misa.fuzzy_choices source prefix)
      (let [result {}]
        (each [_ candidate (ipairs source)]
          (when (= (candidate.value:sub 1 (length prefix)) prefix)
            (tset result (+ (length result) 1) candidate)))
        (table.sort result (fn [left right] (< left.value right.value)))
        result)))

(fn registrations.reg_tool [tool]
  (open)
  (assert (and (and (= (type tool) :table) (= (type tool.name) :string))
               (not= tool.name ""))
          "tool.name must be a nonempty string")
  (assert (= (type tool.description) :string)
          "tool.description must be a string")
  (assert (and (= (type tool.input_schema) :table)
               (= tool.input_schema.type :object))
          "tool.input_schema must be an object schema")
  (assert (and (= (type tool.effect) :string) (not= tool.effect ""))
          "tool.effect must be a nonempty string")
  (assert (= (. tool-by-name tool.name) nil) "duplicate tool")
  (tset tool-by-name tool.name tool)
  (tset tools (+ (length tools) 1) tool)
  nil)

(fn misa.tools [] tools)

(fn misa.tool [name] (. tool-by-name name))

;; The MCP bridge asks Lua for schemas and translates calls through the same

;; registered tool/effect policy used by the interactive agent.

(fn misa._mcp_tools [] tools)

(fn misa._mcp_tool_effect [name arguments id]
  (assert sealed "registrations are not sealed")
  (local tool (assert (. tool-by-name name)
                      (.. "unknown tool: " (tostring name))))
  (assert (= (type arguments) :table) "tool arguments must be an object")
  (runtime-effect {:type tool.effect})
  (local translator
         (assert (. fx-fns tool.effect)
                 (.. "tool effect has no translator: " tool.effect)))
  (local translated (translator {: arguments
                                 : name
                                 :request_id :mcp
                                 :tool_call_id id
                                 :type tool.effect}
                                {:argv base-context.argv
                                 :config base-context.config
                                 :terminal {:columns 80
                                            :interactive false
                                            :lines 24}}
                                db))
  (assert (and (= (type translated) :table) (= (type translated.type) :string))
          "MCP tool translator must return one native effect")
  (runtime-effect translated))

(fn finite [value]
  (and (and (= value value) (not= value math.huge)) (not= value (- math.huge))))

;; One working copy gives a transaction exclusive state without repeatedly

;; cloning immutable configuration and coeffects.

(fn clone [value active depth]
  (set-forcibly! depth (or depth 0))
  (assert (<= depth MAX_DEPTH) "maximum state nesting depth exceeded")
  (local kind (type value))
  (if (or (or (or (= value misa.json_null) (= kind :nil)) (= kind :boolean))
          (= kind :string))
      value
      (if (= kind :number) (do
                             (assert (finite value) "non-finite number")
                             value)
          (do
            (assert (= kind :table) "state must contain only data")
            (set-forcibly! active (or active {}))
            (assert (not (. active value)) "cyclic state")
            (tset active value true)
            (local result {})
            (each [key item (pairs value)]
              (local key-kind (type key))
              (assert (or (= key-kind :string)
                          (and (and (and (= key-kind :number) (finite key))
                                    (>= key 1))
                               (= (% key 1) 0)))
                      "invalid state key")
              (tset result key (clone item active (+ depth 1))))
            (tset active value nil)
            result))))

;; Reducers currently receive a private mutable draft because that keeps the
;; extension API pleasant during the migration.  Reconcile turns that draft
;; back into a persistent value: unchanged branches are returned from the
;; previous state, while changed branches are rebuilt recursively.
(fn reconcile [previous next depth]
  (set-forcibly! depth (or depth 0))
  (assert (<= depth MAX_DEPTH) "maximum state nesting depth exceeded")
  (if (= previous next) previous
      (if (or (or (= next misa.json_null) (= (type next) :nil))
              (= (type next) :boolean) (= (type next) :string)
              (= (type next) :number))
          next
          (if (not= (type next) :table)
              next
              (if (not= (type previous) :table)
                  (clone next nil depth)
                  (do
                    (var changed false)
                    (local result {})
                    (each [key value (pairs next)]
                      (local old-value (. previous key))
                      (local new-value (reconcile old-value value (+ depth 1)))
                      (when (not= new-value old-value) (set changed true))
                      (tset result key new-value))
                    (each [key _ (pairs previous)]
                      (when (= (. next key) nil) (set changed true)))
                    (if changed result previous)))))))


;; Public to projection infrastructure only. Projection models may contain

;; registered callbacks (for example picker indexing policy), so preserve

;; functions while recursively copying every table they could otherwise mutate.

(fn snapshot [value active depth]
  (set-forcibly! depth (or depth 0))
  (assert (<= depth MAX_DEPTH) "maximum projection nesting depth exceeded")
  (local kind (type value))
  (if (or (or (or (or (= kind :function) (= value misa.json_null))
                  (= kind :nil)) (= kind :boolean)) (= kind :string))
      value
      (if (= kind :number) (do
                             (assert (finite value) "non-finite number")
                             value)
          (do
            (assert (= kind :table)
                    "projection input must contain only data or callbacks")
            (set-forcibly! active (or active {}))
            (assert (not (. active value)) "cyclic projection input")
            (tset active value true)
            (local result {})
            (each [key item (pairs value)]
              (tset result key (snapshot item active (+ depth 1))))
            (tset active value nil)
            result))))

(fn misa.snapshot [value] (snapshot value))

(fn append [destination ___values___]
  (assert (= (type ___values___) :table) "fx must be an array")
  (for [i 1 (length ___values___)]
    (assert (= (type (. ___values___ i)) :table) "fx entries must be tables")
    (tset destination (+ (length destination) 1) (. ___values___ i)))
  nil)

;; Setup is an ordered effect program, interpreted before event dispatch begins.

(fn install-service [effect]
  (assert (and (= (type effect.name) :string) (not= effect.name ""))
          "service name must be nonempty")
  (assert (not= effect.value nil) "service value must be non-nil")
  (assert (not (effect.name:match "^_")) "private service name is reserved")
  (local path {})
  (each [part (effect.name:gmatch "[^.]+")]
    (tset path (+ (length path) 1) part))
  (assert (= (table.concat path ".") effect.name) "invalid service path")
  (var target misa)
  (for [index 1 (- (length path) 1)]
    (local key (. path index))
    (when (= (. target key) nil) (tset target key {}))
    (set target (. target key))
    (assert (= (type target) :table) "service namespace must be a table"))
  (local key (. path (length path)))
  (assert (= (. target key) nil) (.. "service already installed: " effect.name))
  (tset target key effect.value)
  nil)

(local builtin-setup {:register/event (fn [e]
                                        (registrations.reg_event e.name
                                                                 e.handler))
                      :register/interceptor (fn [e]
                                              (registrations.reg_interceptor e.value))
                      :register/cofx (fn [e]
                                       (registrations.reg_cofx e.name e.handler))
                      :register/fx (fn [e]
                                     (registrations.reg_fx e.name e.handler))
                      :register/view (fn [e] (registrations.reg_view e.handler))
                      :register/view-layer (fn [e]
                                             (registrations.reg_view_layer e.id
                                                                           e.handler))
                      :register/sub (fn [e]
                                      (registrations.reg_sub e.value))
                      :register/request-options-serializer (fn [e]
                                                             (registrations.reg_request_options_serializer e.id
                                                                                                           e.serializer))
                      :register/model (fn [e] (registrations.reg_model e.value))
                      :register/command (fn [e]
                                          (registrations.reg_command e.value))
                      :register/action (fn [e]
                                         (registrations.reg_action e.value))
                      :register/keybinding (fn [e]
                                             (registrations.reg_keybinding e.value))
                      :register/auth-provider (fn [e]
                                                (registrations.reg_auth_provider e.value))
                      :register/tool (fn [e] (registrations.reg_tool e.value))
                      :register/completion (fn [e]
                                             (registrations.reg_completion e.group
                                                                           e.value))
                      :register/service install-service
                      :register/setup-effect (fn [e]
                                               (assert (and (= (type e.name)
                                                               :string)
                                                            (not= e.name ""))
                                                       "setup effect name must be nonempty")
                                               (assert (e.name:match :^register/.+)
                                                       "setup effect names must use the register/ namespace")
                                               (assert (= (type e.handler)
                                                          :function)
                                                       "setup effect needs a handler")
                                               (assert (= (. setup-handlers
                                                             e.name)
                                                          nil)
                                                       "duplicate setup effect")
                                               (assert (= (. fx-fns e.name) nil)
                                                       "setup effect conflicts with runtime translator")
                                               (tset setup-handlers e.name
                                                     e.handler)
                                               nil)})

(each [name handler (pairs builtin-setup)]
  (tset setup-handlers name handler))

;; Primitive state reads keep ordinary subscriptions declarative.  Feature
;; extensions should register named projections rather than reaching into
;; arbitrary state from their views.
(registrations.reg_sub {:id :db
                        :read (fn [state] state)})
(registrations.reg_sub {:id :db/path
                        :read (fn [state query]
                                (var value state)
                                (for [index 2 (length query)]
                                  (if (= value nil)
                                      (lua "return nil")
                                      (set value (. value (. query index)))))
                                value)})

(fn misa.has_setup_effect [name] (not= (. setup-handlers name) nil))

(fn setup-result [result depth budget]
  (assert (<= depth MAX_DEPTH) "setup effect expansion is too deep")
  (when (not= result nil)
    (assert (and (= (type result) :table) (= (type result.fx) :table))
            "setup must return nil or a table with an fx array")
    (each [key (pairs result)]
      (assert (= key :fx) "setup result only supports fx"))
    (local count (length result.fx))
    (each [key (pairs result.fx)]
      (assert (and (and (= (type key) :number) (= (% key 1) 0))
                   (and (>= key 1) (<= key count)))
              "setup fx must be a dense array"))
    (for [index 1 count]
      (assert (not= (. result.fx index) nil) "setup fx must be a dense array"))
    (for [index 1 count]
      (local effect (. result.fx index))
      (assert (and (= (type effect) :table) (= (type effect.type) :string))
              "setup effect must have a type")
      (set budget.remaining (- budget.remaining 1))
      (assert (>= budget.remaining 0) "too many setup effects")
      (local handler
             (assert (. setup-handlers effect.type)
                     (.. "unknown setup effect: " effect.type)))
      (setup-result (handler effect) (+ depth 1) budget)))
  nil)

(fn misa._setup_effects [result]
  (open)
  (setup-result result 0 {:remaining 100000}))

(fn misa._setup [extension context]
  (open)
  (assert (= (type extension) :table) "extension must be a table")
  (when (not= extension.setup nil)
    (assert (= (type extension.setup) :function)
            "extension setup must be a function")
    (misa._setup_effects (extension.setup context)))
  nil)

(fn misa._seal [context] (set sealed true) (set base-context context) nil)

(fn misa._dispatch [event terminal clock]
  (assert (and sealed (not dispatching)) "invalid dispatch state")
  (assert (= pending-db nil) "previous transaction was not committed")
  (assert (and (and (= (type event) :table) (= (type event.type) :string))
               (not= event.type ""))
          "event.type must be a nonempty string")
  (set dispatching true)
  (set active-sub-scope (subscription-scope.fork))
  (local (ok native projection)
         (xpcall (fn []
                   (assert (and (and (= (type clock) :table)
                                     (= (type clock.wall_ms) :number))
                                (= (type clock.monotonic_ms) :number))
                           "native clock coeffect is missing")
                   (local cofx {:argv base-context.argv
                                : clock
                                :config base-context.config
                                : terminal})
                   (local working (clone db))
                   (each [_ name (ipairs cofx-order)]
                     (tset cofx name ((. cofx-fns name) cofx event working)))
                   (var tx {: cofx :db working : event :fx {}})

                   (fn validate [value]
                     (assert (and (and (and (= (type value) :table)
                                            (= (type value.db) :table))
                                       (= (type value.cofx) :table))
                                  (= (type value.fx) :table))
                             "invalid interceptor transaction")
                     nil)

                   (for [i 1 (length interceptors)]
                     (local before (. interceptors i :before))
                     (when before
                       (set tx (or (before tx) tx))
                       (validate tx)))
                   (each [_ handler (ipairs (or (. events tx.event.type) {}))]
                     (local result (handler tx.db tx.event tx.cofx))
                     (assert (or (= result nil) (= (type result) :table))
                             "event handler result must be a table")
                     (when result
                       (when (not= result.patch nil)
                         (assert (= result.db nil)
                                 "handler cannot return both db and patch")
                         (set tx.db (misa.patch tx.db result.patch)))
                       (when (not= result.db nil)
                         (assert (= (type result.db) :table)
                                 "handler db must be a table")
                         (set tx.db result.db))
                       (when (not= result.fx nil) (append tx.fx result.fx))))
                   (for [i (length interceptors) 1 (- 1)]
                     (local after (. interceptors i :after))
                     (when after
                       (set tx (or (after tx) tx))
                       (validate tx)))
                   (set tx.db (reconcile db tx.db))
                   (local effects {})
                   (each [_ effect (ipairs tx.fx)]
                     (local kind (. (runtime-effect effect) :type))
                     (assert (and (= (type kind) :string) (not= kind ""))
                             "effect.type must be a nonempty string")
                     (local translator (. fx-fns kind))
                     (if translator
                         (do
                           (var translated nil)
                           (local tool
                                  (and effect.name (. tool-by-name effect.name)))
                           (if (and (and tool (= tool.effect kind))
                                    (= (type effect.tool_call_id) :string))
                               (let [(translated-ok value) (pcall translator
                                                                  effect tx.cofx
                                                                  tx.db)]
                                 (set translated
                                      (or (and translated-ok value)
                                          {:event {:is_error true
                                                   :text (tostring value)
                                                   :tool_call_id effect.tool_call_id
                                                   :type :tool/result}
                                           :type :dispatch})))
                               (set translated
                                    (translator effect tx.cofx tx.db)))
                           (assert (= (type translated) :table)
                                   "fx translator must return a table")
                           (if translated.type
                               (tset effects (+ (length effects) 1) translated)
                               (append effects translated)))
                         (tset effects (+ (length effects) 1) effect)))
                   (each [_ effect (ipairs effects)] (runtime-effect effect))
                   ;; The reconciled database is persistent by convention:
                   ;; views receive it directly so subscription inputs retain
                   ;; identity across transactions.  Views are projections
                   ;; and must not mutate it.
                   (var frame misa.json_null)
                   (when view
                     (set frame (view tx.db (clone tx.cofx)))
                     (assert (= (type frame) :table) "view must return a table"))
                   (set pending-sub-scope active-sub-scope)
                   (set pending-db tx.db)
                   (values effects frame)) traceback))
  (set dispatching false)
  (when (not ok) (active-sub-scope.close))
  (set active-sub-scope nil)
  (when (not ok) (error native 0))
  (values native projection))

(fn misa._commit [] (assert (not= pending-db nil) "no transaction to commit")
  (set (db pending-db) (values pending-db nil))
  (subscription-scope.close)
  (set (subscription-scope pending-sub-scope) (values pending-sub-scope nil))
  nil)

(fn misa._rollback []
  (assert (not dispatching) "cannot roll back while dispatching")
  (when pending-sub-scope (pending-sub-scope.close))
  (set (pending-db pending-sub-scope) (values nil nil))
  nil)

;; Extensions are trusted policy. Keep ordinary Lua loading/composition, while

;; reserving terminal output, process termination, and native-library loading

;; to Zig-owned effects.

(when package (set package.loadlib nil)
  (set (package.loaded.io package.loaded.os package.loaded.debug)
       (values nil nil nil))
  (set (package.loaded.ffi package.loaded.jit) (values nil nil))
  (set (package.preload.ffi package.preload.jit) (values nil nil))
  (tset package.loaders 3 nil)
  (tset package.loaders 4 nil))

(global (os io print ffi jit debug) (values nil nil nil nil nil nil))

nil
