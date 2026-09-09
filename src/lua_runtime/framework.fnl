;; Trusted event framework, embedded by Zig.

(local traceback debug.traceback)
(local locale-date os.date)
;; Initialize time formatting from LC_TIME/LANG before hiding the OS library.
(os.setlocale "" :time)

(local (events event-routes route-ids) (values {} {} {}))

(local (cofx-fns cofx-order fx-fns) (values {} {} {}))

(local (models model-by-id tools tool-by-name) (values {} {} {} {}))

(local (commands command-by-name) (values {} {}))

(local (actions action-by-id) (values {} {}))

(local (completions completion-values) (values {} {}))

(local (keybindings keybinding-ids) (values {} {}))

(local (auth-providers auth-by-id auth-by-model) (values {} {} {}))

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
(var projection-scope (subscription-registry.scope))
(var pending-projection-scope nil)
(var projection-entries {})
(var pending-projection-entries nil)
(var presentation {})
(var pending-presentation nil)
(var projecting false)
(var installed-catalogs nil)
(var installed-catalog-entries nil)
(local empty-catalog {})

(var (view sealed dispatching db pending-db base-context)
     (values nil false false {} nil nil))

(local MAX_DEPTH 128)

(global misa {:json-null {}
              :actions {}
              :auth {}
              :commands {}
              :keybindings {}
              :models {}
              :projections {}
              :request-options {}
              :subscriptions {}
              :time {}
              :tools {}
              :ui {}})

;; Format an explicit instant; current time still comes from the clock coeffect.
(fn misa.time.local-datetime [seconds]
  "Format an explicit instant using the configured locale."
  (local (ok value) (pcall locale-date "%x %X %Z" seconds))
  (when ok value))

(local state-updates ((require :misa.runtime.state) misa.json-null))
(set misa.delete state-updates.delete)
(set misa.replace state-updates.replace)
(set misa.patch state-updates.patch)

(local registrations {})

(fn runtime-effect [effect]
  (assert (and (and (= (type effect) :table) (= (type effect.type) :string))
               (not= effect.type "")) "effect must have a type")
  (assert (not (effect.type:match :^register/))
          "registration declarations cannot run as effects")
  effect)

(fn open [] (assert (not sealed) "registrations are sealed") nil)

(fn registrations.event! [name handler]
  (open)
  (assert (and (= (type name) :string) (not= name "")))
  (assert (= (type handler) :function))
  (local handlers (or (. events name) {}))
  (tset events name handlers)
  (tset handlers (+ (length handlers) 1) handler)
  nil)

(fn registrations.event-route! [value]
  (open)
  (assert (and (= (type value) :table) (= (type value.id) :string)
               (not= value.id "") (= (type value.event) :string)
               (not= value.event "") (= (type value.priority) :number)
               (> value.priority (- math.huge)) (< value.priority math.huge)
               (= value.priority (math.floor value.priority))
               (= (type value.context) :table)
               (= (type value.resolve) :function))
          "invalid event route")
  (assert (not (. route-ids value.id)) "duplicate event route")
  (tset route-ids value.id true)
  (local routes (or (. event-routes value.event) []))
  (tset event-routes value.event routes)
  (table.insert routes value)
  (table.sort routes (fn [a b]
                       (if (= a.priority b.priority) (< a.id b.id)
                           (> a.priority b.priority))))
  nil)

(fn registrations.cofx! [name handler]
  (open)
  (assert (and (= (type name) :string) (not= name "")))
  (assert (and (and (and (not= name :config) (not= name :argv))
                    (not= name :terminal)) (not= name :clock)
               (not= name :host)))
  (assert (and (= (type handler) :function) (= (. cofx-fns name) nil))
          "duplicate cofx")
  (tset cofx-fns name handler)
  (tset cofx-order (+ (length cofx-order) 1) name)
  nil)

(fn registrations.fx! [name handler]
  (open)
  (assert (and (= (type name) :string) (not= name "")))
  (runtime-effect {:type name})
  (assert (and (= (type handler) :function) (= (. fx-fns name) nil))
          "duplicate fx")
  (tset fx-fns name handler)
  nil)

(fn registrations.view! [handler]
  (open)
  (assert (and (= (type handler) :function) (= view nil))
          "view already registered")
  (set view handler)
  nil)

(fn registrations.view-layer! [id handler]
  (open)
  (assert (and (= (type id) :string) (not= id ""))
          "view layer ID must be nonempty")
  (assert (and (= (type handler) :function) (not (. view-layer-ids id)))
          "duplicate view layer")
  (tset view-layer-ids id true)
  (tset view-layers (+ (length view-layers) 1) handler)
  nil)

(fn registrations.sub! [definition]
  (open)
  (subscription-registry.register definition))

(fn misa.subscriptions.scope [capacity]
  "Create a bounded subscription cache owned by its caller."
  (subscription-registry.scope capacity))

(fn misa.sub [state query]
  "Resolve a subscription against immutable state in the active transaction."
  ((. (or active-sub-scope subscription-scope) :query) state query))

(fn misa.ui.layers [state cofx]
  "Project registered layers in their declared order."
  (let [result {}]
    (each [_ project (ipairs view-layers)]
      (local layer (project state cofx))
      (assert (or (= layer nil) (= (type layer) :table))
              "view layer must be a table")
      (when layer
        (tset result (+ (length result) 1) layer)))
    result))

(fn scalar? [value]
  (let [kind (type value)]
    (or (or (= kind :string) (= kind :number)) (= kind :boolean))))

(fn registrations.request-options-serializer! [id serializer]
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

(fn misa.request-options.serializable? [id name]
  "Report whether a serializer accepts an option name."
  (let [serializer (. request-serializers id)]
    (and (not= serializer nil) (= (serializer.accepts name) true))))

(fn misa.request-options.serialize [id options target]
  "Return a request assembled from serializer patches without changing TARGET."
  (local serializer (assert (. request-serializers id)
                            (.. "unknown request option serializer: "
                                (tostring id))))
  (assert (= (type options) :table) "request options must be a table")
  (var result (or target {}))
  (each [name value (pairs options)]
    (assert (= (serializer.accepts name) true)
            (.. "request option is not serializable: " (tostring name)))
    (local patch (serializer.serialize name value))
    (assert (= (type patch) :table)
            (.. "request option serializer must return a patch: "
                (tostring name)))
    (set result (misa.patch result patch)))
  result)

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
            (assert (scalar? choice)
                    "request option choices must be scalar values")
            (local key (.. (type choice) ":" (tostring choice)))
            (assert (not (. seen key)) "request option choices must be unique")
            (tset seen key true))
          (assert (or (= option.default nil) (scalar? option.default))
                  "request option default must be a scalar value")
          (when (and (not= option.default nil) choices)
            (local key (.. (type option.default) ":" (tostring option.default)))
            (assert (. seen key)
                    "request option default must be one of its choices")))
        nil)))

(fn registrations.model! [model]
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

(fn misa.models.all []
  "Read the borrowed immutable model definitions."
  models)

(fn misa.models.lookup [id]
  "Look up a borrowed immutable model by ID."
  (. model-by-id id))

(fn registrations.command! [command]
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
  (assert (or (and (= command.choice_available nil)
                   (= command.choice_unavailable nil))
              (and (= (type command.choice_available) :function)
                   (= (type command.choice_unavailable) :string)
                   (not= command.choice_unavailable "")))
          "command choice availability requires a predicate and unavailable event")
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

(fn misa.commands.all []
  "Read the borrowed immutable command definitions."
  commands)

(fn misa.commands.lookup [name]
  "Look up a borrowed immutable command by name."
  (. command-by-name name))

;; UI actions are discoverable invocations, separate from conversational slash commands.

(fn registrations.action! [action]
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
    (registrations.keybinding! {:action action.id
                                :context action.binding.context
                                :default (or action.keys {})}))
  (assert (and (and (= (type action.binding) :table)
                    (= (type action.binding.context) :string))
               (= (type action.binding.action) :string))
          "action binding must identify context and action")
  (tset action-by-id action.id action)
  (tset actions (+ (length actions) 1) action)
  nil)

(fn misa.actions.all []
  "Read the borrowed immutable action definitions."
  actions)

(fn misa.actions.lookup [id]
  "Look up a borrowed immutable action by ID."
  (. action-by-id id))

;; Keybinding declarations are data. The keybindings extension owns input

;; normalization and configuration policy; feature extensions only name actions.

(fn registrations.keybinding! [binding]
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

(fn misa.keybindings.all []
  "Read the borrowed immutable keybinding declarations."
  keybindings)

(fn registrations.completion! [group candidate]
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

(fn registrations.auth-provider! [provider]
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
  (registrations.completion! :auth-provider
                             {:description provider.description
                              :label provider.label
                              :value provider.id})
  nil)

(fn misa.auth.providers []
  "Read the borrowed immutable authentication providers."
  auth-providers)

(fn misa.auth.provider [id]
  "Look up an authentication provider by ID."
  (. auth-by-id id))

(fn misa.auth.for-model [id]
  "Look up the authentication provider for a model provider."
  (. auth-by-model id))

(fn misa.commands.completions [command prefix state]
  "Resolve completion candidates for a command and prefix."
  (assert (and (= (type command) :table) (= (type prefix) :string))
          "invalid completion request")
  (local source (or (or (and command.complete (command.complete prefix state))
                        (. completions command.completion))
                    {}))
  (assert (= (type source) :table) "command completer must return an array")
  (each [_ candidate (ipairs source)]
    (assert (and (= (type candidate) :table) (= (type candidate.value) :string))
            "invalid completion candidate"))
  (if (and misa.fuzzy misa.fuzzy.choices) (misa.fuzzy.choices source prefix)
      (let [result {}]
        (each [_ candidate (ipairs source)]
          (when (= (candidate.value:sub 1 (length prefix)) prefix)
            (tset result (+ (length result) 1) candidate)))
        (table.sort result (fn [left right] (< left.value right.value)))
        result)))

(fn registrations.tool! [tool]
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

(fn misa.tools.all []
  "Read the borrowed immutable tool definitions."
  tools)

(fn misa.tools.lookup [name]
  "Look up a borrowed immutable tool definition by name."
  (. tool-by-name name))

;; The MCP bridge asks Lua for schemas and translates calls through the same

;; registered tool/effect policy used by the interactive agent.

(fn misa._mcp_tools []
  "Expose registered tool schemas to the native MCP bridge."
  tools)

(fn coeffects [event terminal clock state]
  (local result {:argv base-context.argv
                 :host base-context.host
                 :config base-context.config
                 : terminal
                 : clock
                 : presentation})
  (each [_ name (ipairs cofx-order)]
    (tset result name ((. cofx-fns name) result event state)))
  result)

(fn translate-effect [effect cofx state]
  (local kind (. (runtime-effect effect) :type))
  (local translator (. fx-fns kind))
  (if (not translator) effect (let [tool (and effect.name
                                              (. tool-by-name effect.name))
                                    translated (if (and tool
                                                        (= tool.effect kind)
                                                        (= (type effect.tool_call_id)
                                                           :string))
                                                   (let [(ok value) (pcall translator
                                                                           effect
                                                                           cofx
                                                                           state)]
                                                     (if (and ok value) value
                                                         {:type :dispatch
                                                          :event {:type :tool/result
                                                                  :is_error true
                                                                  :tool_call_id effect.tool_call_id
                                                                  :text (tostring value)}}))
                                                   (translator effect cofx
                                                               state))]
                                (assert (= (type translated) :table)
                                        "fx translator must return a table")
                                translated)))

(fn misa._mcp_tool_effect [name arguments id terminal clock]
  "Translate an MCP tool call through the registered effect and coeffect policy."
  (assert sealed "registrations are not sealed")
  (local tool (assert (. tool-by-name name)
                      (.. "unknown tool: " (tostring name))))
  (assert (= (type arguments) :table) "tool arguments must be an object")
  (assert (. fx-fns tool.effect)
          (.. "tool effect has no translator: " tool.effect))
  (local effect {: arguments
                 : name
                 :request_id :mcp
                 :tool_call_id id
                 :type tool.effect})
  (local translated
         (translate-effect effect (coeffects effect terminal clock db) db))
  (assert (= (type translated.type) :string)
          "MCP tool translator must return one native effect")
  (runtime-effect translated))

(fn finite? [value]
  (and (and (= value value) (not= value math.huge)) (not= value (- math.huge))))

(fn snapshot [value ancestors level]
  (local depth (or level 0))
  (assert (<= depth MAX_DEPTH) "maximum projection nesting depth exceeded")
  (local kind (type value))
  (if (or (or (or (or (= kind :function) (= value misa.json-null))
                  (= kind :nil)) (= kind :boolean)) (= kind :string))
      value
      (if (= kind :number) (do
                             (assert (finite? value) "non-finite number")
                             value)
          (do
            (assert (= kind :table)
                    "projection input must contain only data or callbacks")
            (local active (or ancestors {}))
            (assert (not (. active value)) "cyclic projection input")
            (tset active value true)
            (local result {})
            (each [key item (pairs value)]
              (tset result key (snapshot item active (+ depth 1))))
            (tset active value nil)
            result))))

(fn misa.snapshot [value]
  "Copy projection data while preserving registered callback identities."
  (snapshot value))

(fn misa.projections.publish [id value]
  "Publish data for input handlers when the containing frame is accepted."
  (assert projecting
          "presentation data can only be published during projection")
  (assert (and (= (type id) :string) (not= id ""))
          "presentation publication requires an id")
  (tset pending-presentation id (snapshot value))
  nil)

(fn append [destination items]
  (assert (= (type items) :table) "fx must be an array")
  (for [i 1 (length items)]
    (assert (= (type (. items i)) :table) "fx entries must be tables")
    (tset destination (+ (length destination) 1) (. items i)))
  nil)

(fn install-service [definition]
  (assert (and (= (type definition.name) :string) (not= definition.name ""))
          "service name must be nonempty")
  (assert (not= definition.value nil) "service value must be non-nil")
  (assert (not (definition.name:match "^_")) "private service name is reserved")
  (local path {})
  (each [part (definition.name:gmatch "[^.]+")]
    (tset path (+ (length path) 1) part))
  (assert (= (table.concat path ".") definition.name) "invalid service path")
  (var target misa)
  (for [index 1 (- (length path) 1)]
    (local key (. path index))
    (when (= (. target key) nil) (tset target key {}))
    (set target (. target key))
    (assert (= (type target) :table) "service namespace must be a table"))
  (local key (. path (length path)))
  (assert (= (. target key) nil)
          (.. "service already installed: " definition.name))
  (tset target key definition.value)
  nil)

;; A projection owner declares its invalidation inputs. Its render callback may
;; compose other projections and subscriptions; only accepted frames retain
;; these immutable results. Calls from model handlers compute without publishing
;; speculative presentation caches.
(fn same-fields? [a b]
  (and a b (do
             (each [key value (pairs a)]
               (when (not= value (. b key)) (lua "return false")))
             (each [key value (pairs b)]
               (when (not= value (. a key)) (lua "return false")))
             true)))

(fn install-projection [definition]
  (assert (and (= (type definition.inputs) :function)
               (= (type definition.render) :function))
          "projection requires inputs and render callbacks")
  (install-service {:name definition.name
                    :value (fn [state context]
                             (local inputs (definition.inputs state context))
                             (assert (= (type inputs) :table)
                                     "projection inputs must be a table")
                             (local previous
                                    (. (if projecting
                                           pending-projection-entries
                                           projection-entries)
                                       definition.name))
                             (if (and previous
                                      (same-fields? previous.inputs inputs)
                                      (same-fields? previous.context
                                                    (or context {})))
                                 previous.value
                                 (let [value (definition.render state context)
                                       entry {:inputs (collect [key item (pairs inputs)]
                                                        key
                                                        item)
                                              :context (collect [key item (pairs (or context
                                                                                     {}))]
                                                         key
                                                         item)
                                              : value}]
                                   (when projecting
                                     (tset pending-projection-entries
                                           definition.name entry))
                                   value)))}))

;; Primitive state reads keep ordinary subscriptions declarative.  Feature
;; extensions should register named projections rather than reaching into
;; arbitrary state from their views.
(registrations.sub! {:id :db :read (fn [state] state)})

(registrations.sub! {:id :db/path
                     :read (fn [state query]
                             (var value state)
                             (for [index 2 (length query)]
                               (if (= value nil)
                                   (lua "return nil")
                                   (set value (. value (. query index)))))
                             value)})

(fn misa.catalog [kind]
  "Read a borrowed immutable catalog from the installed application."
  (assert installed-catalogs "application definitions are not installed")
  (or (. installed-catalogs kind) empty-catalog))

(fn misa.catalog-entries [kind]
  "Read borrowed catalog entries in deterministic priority and ID order."
  (assert installed-catalog-entries "application definitions are not installed")
  (or (. installed-catalog-entries kind) empty-catalog))

(fn misa.compose [applications]
  "Compose application values; later named definitions replace earlier entries.

Deleted definitions remain explicitly disabled until installation."
  (local definitions {})
  (local modules {})
  (var config {})
  (each [_ application (ipairs applications)]
    (assert (= (type application) :table) "application must be a table")
    (set config (misa.patch config (or application.config {})))
    (each [id value (pairs (or application.modules {}))]
      (tset modules id (if (= value misa.delete) nil value)))
    (each [kind entries (pairs (or application.definitions {}))]
      (assert (= (type entries) :table) "definition catalog must be a table")
      (when (not (. definitions kind)) (tset definitions kind {}))
      (each [id value (pairs entries)]
        (tset (. definitions kind) id value))))
  {: config : definitions : modules})

(fn ordered-definitions [catalog]
  (local rows (icollect [id value (pairs (or catalog {}))]
                {: id : value}))
  (table.sort rows (fn [a b]
                     (local ap (if (= (type a.value) :table)
                                   (or a.value.priority 0)
                                   0))
                     (local bp (if (= (type b.value) :table)
                                   (or b.value.priority 0)
                                   0))
                     (if (= ap bp) (< a.id b.id) (< ap bp))))
  rows)

(fn misa._install [definitions context]
  "Validate and install one composed application, then seal its definitions."
  (open)
  (assert (= installed-catalogs nil) "application already installed")
  (assert (= (type definitions) :table)
          "application definitions must be a table")
  (local enabled {})
  (each [kind entries (pairs definitions)]
    (assert (and (= (type kind) :string) (= (type entries) :table))
            "invalid definition catalog")
    (local catalog {})
    (each [id value (pairs entries)]
      (assert (and (= (type id) :string) (not= id ""))
              "definition ID must be a nonempty string")
      (when (not= value misa.delete) (tset catalog id value)))
    (tset enabled kind catalog))
  (local catalogs (snapshot enabled))
  (each [kind validate (pairs (or catalogs.validators {}))]
    (assert (= (type validate) :function)
            "catalog validator must be a function")
    (each [_ entry (ipairs (ordered-definitions (. catalogs kind)))]
      (validate entry.id entry.value)))
  (set installed-catalogs catalogs)
  (set installed-catalog-entries
       (collect [kind entries (pairs catalogs)] kind
         (ordered-definitions entries)))
  (local installers
         {:services (fn [id value] (install-service {:name id : value}))
          :projections (fn [id value]
                         (install-projection {:name id
                                              :inputs value.inputs
                                              :render value.render}))
          :subscriptions (fn [id value]
                           (registrations.sub! (misa.patch value {: id})))
          :coeffects registrations.cofx!
          :effects registrations.fx!
          :serializers registrations.request-options-serializer!
          :events (fn [_ value]
                    (registrations.event! value.event value.handler))
          :routes (fn [id value]
                    (registrations.event-route! (misa.patch value {: id})))
          :views (fn [_ value] (registrations.view! value))
          :view-layers (fn [id value]
                         (registrations.view-layer! id
                                                    (if (= (type value) :table)
                                                        value.handler
                                                        value)))
          :models (fn [id value]
                    (registrations.model! (misa.patch value {: id})))
          :commands (fn [id value]
                      (registrations.command! (misa.patch value {:name id})))
          :actions (fn [id value]
                     (registrations.action! (misa.patch value {: id})))
          :keybindings (fn [_ value] (registrations.keybinding! value))
          :auth-providers (fn [id value]
                            (registrations.auth-provider! (misa.patch value
                                                                      {: id})))
          :tools (fn [id value]
                   (registrations.tool! (misa.patch value {:name id})))
          :completions (fn [_ value]
                         (registrations.completion! value.group value.value))})
  (each [_ kind (ipairs [:services
                         :projections
                         :subscriptions
                         :coeffects
                         :effects
                         :serializers
                         :events
                         :routes
                         :views
                         :view-layers
                         :models
                         :commands
                         :actions
                         :keybindings
                         :auth-providers
                         :tools
                         :completions])]
    (each [_ entry (ipairs (ordered-definitions (. catalogs kind)))]
      ((. installers kind) entry.id entry.value)))
  (each [owner requirements (pairs (or catalogs.requirements {}))]
    (assert (= (type requirements) :table)
            "requirements must be an array of service paths")
    (each [_ path (ipairs requirements)]
      (assert (= (type path) :string) "requirement must name a service path")
      (var value misa)
      (each [part (path:gmatch "[^.]+")]
        (set value (and (= (type value) :table) (. value part))))
      (assert (not= value nil) (.. owner " requires " path))))
  (set base-context context)
  (set sealed true)
  nil)

(fn misa._dispatch [event terminal clock]
  "Stage one model transaction and return its native effects."
  (assert (and sealed (not dispatching) (not projecting)
               (= pending-projection-scope nil))
          "invalid dispatch state")
  (assert (= pending-db nil) "previous transaction was not committed")
  (assert (and (and (= (type event) :table) (= (type event.type) :string))
               (not= event.type ""))
          "event.type must be a nonempty string")
  (set dispatching true)
  (set active-sub-scope (subscription-scope.fork))
  (local (ok native) (xpcall (fn []
                               (assert (and (and (= (type clock) :table)
                                                 (= (type clock.wall_ms)
                                                    :number))
                                            (= (type clock.monotonic_ms)
                                               :number))
                                       "native clock coeffect is missing")
                               (local cofx (coeffects event terminal clock db))
                               ;; Routes inspect only their declared source event. A route is
                               ;; a pure choice of semantic event, never a transaction hook.
                               (var routed nil)
                               (var winning-priority nil)
                               (each [_ route (ipairs (or (. event-routes
                                                             event.type)
                                                          []))]
                                 (when (and winning-priority
                                            (< route.priority winning-priority))
                                   (lua :break))
                                 (local context (misa.sub db route.context))
                                 (when (not= context nil)
                                   (local candidate
                                          (route.resolve context event cofx))
                                   (when (not= candidate nil)
                                     (assert (and (= (type candidate) :table)
                                                  (= (type candidate.type)
                                                     :string)
                                                  (not= candidate.type ""))
                                             "event route must return nil or an event")
                                     (assert (= routed nil)
                                             "ambiguous event routes at winning priority")
                                     (set routed candidate)
                                     (set winning-priority route.priority))))
                               (var tx
                                    {: cofx
                                     : db
                                     :event (or routed event)
                                     :fx []})
                               (each [_ handler (ipairs (or (. events
                                                               tx.event.type)
                                                            {}))]
                                 (local result (handler tx.db tx.event tx.cofx))
                                 (assert (or (= result nil)
                                             (= (type result) :table))
                                         "event handler result must be a table")
                                 (when result
                                   (assert (= result.db nil)
                                           "handler must return patch, not db")
                                   (when (not= result.patch nil)
                                     (set tx.db (misa.patch tx.db result.patch)))
                                   (when (not= result.fx nil)
                                     (append tx.fx result.fx))))
                               (local effects {})
                               (each [_ effect (ipairs tx.fx)]
                                 (local translated
                                        (translate-effect effect tx.cofx tx.db))
                                 (if translated.type
                                     (table.insert effects translated)
                                     (append effects translated)))
                               (each [_ effect (ipairs effects)]
                                 (runtime-effect effect))
                               (set pending-sub-scope active-sub-scope)
                               (set pending-db tx.db)
                               effects) traceback))
  (set dispatching false)
  (when (not ok) (active-sub-scope.close))
  (set active-sub-scope nil)
  (when (not ok) (error native 0))
  native)

(fn misa._commit []
  "Accept the staged model transaction independently of presentation."
  (assert (not= pending-db nil) "no transaction to commit")
  (set (db pending-db) (values pending-db nil))
  (subscription-scope.close)
  (set (subscription-scope pending-sub-scope) (values pending-sub-scope nil))
  nil)

(fn misa._rollback []
  "Discard staged model changes and subscription memoization."
  (assert (not dispatching) "cannot roll back while dispatching")
  (when pending-sub-scope (pending-sub-scope.close))
  (set (pending-db pending-sub-scope) (values nil nil))
  nil)

(fn misa._project [terminal clock]
  "Stage a frame from committed state and explicit terminal facts."
  (assert (and sealed (not dispatching) (not projecting) (= pending-db nil)
               (= pending-projection-scope nil))
          "invalid projection state")
  (set projecting true)
  (set active-sub-scope (projection-scope.fork))
  (set pending-projection-entries (collect [key entry (pairs projection-entries)]
                                    key
                                    entry))
  (set pending-presentation
       (collect [key value (pairs presentation)] key value))
  (local (ok frame) (xpcall (fn []
                              (local cofx
                                     {:argv base-context.argv
                                      :config base-context.config
                                      : terminal
                                      : clock
                                      :projecting true})
                              (local result
                                     (if view (view db cofx) misa.json-null))
                              (assert (= (type result) :table)
                                      "view must return a table")
                              result) traceback))
  (set projecting false)
  (if ok (set pending-projection-scope active-sub-scope)
      (do
        (active-sub-scope.close)
        (set pending-projection-entries nil)
        (set pending-presentation nil)))
  (set active-sub-scope nil)
  (when (not ok) (error frame 0))
  frame)

(fn misa._commit_projection []
  "Accept staged presentation caches and published geometry."
  (assert pending-projection-scope "no projection to commit")
  (projection-scope.close)
  (set (projection-scope pending-projection-scope)
       (values pending-projection-scope nil))
  (set (projection-entries pending-projection-entries)
       (values pending-projection-entries nil))
  (set (presentation pending-presentation) (values pending-presentation nil))
  nil)

(fn misa._rollback_projection []
  "Discard staged presentation without changing model state."
  (assert (not projecting) "cannot roll back while projecting")
  (when pending-projection-scope (pending-projection-scope.close))
  (set (pending-projection-scope pending-projection-entries) (values nil nil))
  (set pending-presentation nil)
  nil)

;; Extensions are trusted policy. Keep ordinary Lua loading/composition, while

;; reserving terminal output, process termination, and native-library loading

;; to Zig-owned effects.

(when package
  (when package.loadlib
    (table.remove package.loaders 4)
    (table.remove package.loaders 3))
  (set package.loadlib nil)
  (set (package.loaded.io package.loaded.os package.loaded.debug)
       (values nil nil nil))
  (set (package.loaded.ffi package.loaded.jit) (values nil nil))
  (set (package.preload.ffi package.preload.jit) (values nil nil)))

(global (os io print ffi jit debug) (values nil nil nil nil nil nil))

nil
