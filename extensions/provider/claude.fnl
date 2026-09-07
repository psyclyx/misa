;; Claude Code process transport. Claude owns subscription credentials; Misa

;; owns conversation policy and invokes the same stream-json protocol used by

;; the Agent SDK.

(fn transcript [messages]
  (let [out ["Continue this conversation. Preserve the roles and treat tool results as authoritative."]]
    (each [_ message (ipairs messages)]
      (if (= message.role :user) (tset out (+ (length out) 1) "\nUser:")
          (= message.role :assistant) (tset out (+ (length out) 1)
                                            "\nAssistant:")
          (= message.role :tool) (tset out (+ (length out) 1)
                                       (.. "\nTool result for "
                                           (tostring message.tool_call_id) ":")))
      (each [_ block (ipairs (or message.content {}))]
        (when (= block.type :text)
          (tset out (+ (length out) 1) block.text))))
    (tset out (+ (length out) 1) "\nAssistant:")
    (table.concat out "\n")))

(fn prompt-content [messages]
  (let [blocks [{:text (transcript messages) :type :text}]]
    (each [_ message (ipairs messages)]
      (each [_ block (ipairs (or message.content {}))]
        (when (= block.type :image)
          (tset blocks (+ (length blocks) 1)
                {:source block.source :type :image}))))
    (or (and (= (length blocks) 1) (. blocks 1 :text)) blocks)))

(fn json-string [value]
  (assert (and (= (type value) :string) (not (value:find "%z")))
          "MCP command arguments must be strings without NUL")
  (.. "\""
      (value:gsub "[%z\001-\031\\\"]"
                  (fn [char]
                    (let [escapes {"\b" "\\b"
                                   "\t" "\\t"
                                   "\n" "\\n"
                                   "\f" "\\f"
                                   "\r" "\\r"
                                   "\"" "\\\""
                                   "\\" "\\\\"}]
                      (or (. escapes char)
                          (string.format "\\u%04x" (char:byte)))))) "\""))

(fn mcp-config [command arguments]
  (let [encoded {}]
    (each [_ argument (ipairs arguments)]
      (tset encoded (+ (length encoded) 1) (json-string argument)))
    (.. "{\"mcpServers\":{\"misa\":{\"type\":\"stdio\",\"command\":"
        (json-string command) ",\"args\":[" (table.concat encoded ",") "]}}}")))

(fn mcp-problem [record]
  (when (and (= record.type :system) (= record.subtype :init))
    (accumulate [problem nil _ server (ipairs (or record.mcp_servers []))
                 &until problem]
      (when (and (= server.name :misa)
                 (or (= server.status :failed) (= server.status :needs-auth)))
        (.. "Claude could not connect to Misa's MCP tools (" server.status ").")))))


(fn emit [id type data]
  {:type :dispatch :event (misa.patch (or data {}) {: id : type})})
(fn delta [id value] (emit id :agent/stream-delta {:delta value}))
(fn fresh [] {:message_seq 0 :messages {} :tools {} :block_tools {} :tool_results {}
              :result false :saw_content false})
(fn message-key [state] (or state.current_message "seq:0"))
(fn block-key [state index] (.. (message-key state) ":" (tostring index)))

(fn body [state index value fx id]
  (local key (message-key state))
  (local slot (tostring index))
  (local finalized (and (. state.messages key) (. state.messages key :final_blocks)
                        (. state.messages key :final_blocks slot)))
  (if (and value (not= value.text "") (not finalized))
      (do
        (table.insert fx (delta id value))
        (misa.patch state {:messages {key {:streamed_blocks {slot true}}}
                           :saw_content true}))
      state))

(fn tool [state index block complete fx id]
  (local call-id (assert block.id "Claude tool call requires an id"))
  (local previous (. state.tools call-id))
  (local key (or (and previous previous.index) (block-key state index)))
  (when (or (not previous) (and complete (not previous.complete)))
    (table.insert fx (delta id {:type :tool_call :execution :provider :index key
                               :id call-id :name block.name
                               :arguments (when complete (or block.input {}))
                               :arguments_json (when (not complete) "")})))
  (misa.patch state {:saw_content true
                     :tools {call-id {:index key :complete (or complete (and previous previous.complete) false)}}
                     :block_tools {(block-key state index) call-id}}))

(fn usage [id raw cost]
  (local value (if (= (type raw) :table) raw {}))
  (emit id :agent/stream-usage
        {:usage {:input_tokens (or value.input_tokens 0) :output_tokens (or value.output_tokens 0)
                 :cache_read_tokens (or value.cache_read_input_tokens 0)
                 :cache_write_tokens (or value.cache_creation_input_tokens 0)
                 :input_includes_cache false :cost_usd cost}}))

(local partials
       {:message_start (fn [state record id]
                         (when (= (type record.message) :table)
                           (local sequence (+ state.message_seq 1))
                           (local key (if record.message.id (.. "id:" record.message.id) (.. "seq:" sequence)))
                           {:state (misa.patch state {:message_seq sequence :current_message key})
                            :fx [(usage id record.message.usage)]}))
        :message_delta (fn [_ record id]
                         {:fx [(emit id :agent/stream-usage
                                     {:stop_reason (and (= (type record.delta) :table) record.delta.stop_reason)
                                      :usage {:output_tokens (or (and (= (type record.usage) :table)
                                                                     record.usage.output_tokens) 0)}})]})
        :content_block_start
        (fn [state record id]
          (when (= (type record.content_block) :table)
            (local block record.content_block)
            (local fx [])
            (local value (if (and (= block.type :text) (= (type block.text) :string) (not= block.text ""))
                             {:type :text :text block.text}
                             (and (= block.type :thinking) (= (type block.thinking) :string) (not= block.thinking ""))
                             {:type :thinking :text block.thinking} nil))
            (local next (body state record.index value fx id))
            {:state (if (= block.type :tool_use) (tool next record.index block false fx id) next) : fx}))
        :content_block_delta
        (fn [state record id]
          (when (= (type record.delta) :table)
            (local value record.delta)
            (local fx [])
            (local content (if (and (= value.type :text_delta) (= (type value.text) :string))
                               {:type :text :text value.text}
                               (and (= value.type :thinking_delta) (= (type value.thinking) :string))
                               {:type :thinking :text value.thinking} nil))
            (local next (body state record.index content fx id))
            (when (= value.type :input_json_delta)
              (local call-id (. state.block_tools (block-key state record.index)))
              (local call (and call-id (. state.tools call-id)))
              (when (not (and call call.complete))
                (table.insert fx (delta id {:type :tool_call :index (or (and call call.index) (block-key state record.index))
                                           :arguments_json_delta (or value.partial_json "")}))))
            {:state (if (= value.type :input_json_delta) (misa.patch next {:saw_content true}) next) : fx}))})

(fn assistant-record [previous record id]
  (when (= (type record.message) :table)
    (local current (message-key previous))
    (local advance (and (not record.message.id) (. previous.messages current)
                        (. previous.messages current :finalized)))
    (local sequence (+ previous.message_seq (if advance 1 0)))
    (local key (if record.message.id (.. "id:" record.message.id)
                  advance (.. "seq:" sequence) current))
    (var state (misa.patch previous {:current_message key :message_seq sequence}))
    (local fx [])
    (each [index block (ipairs (or record.message.content []))]
      (local slot (tostring (- index 1)))
      (local entry (or (. state.messages key) {}))
      (if (= block.type :tool_use)
          (set state (tool state (- index 1) block true fx id))
          (and (not (and entry.streamed_blocks (. entry.streamed_blocks slot)))
               (not (and entry.final_blocks (. entry.final_blocks slot))))
          (let [value (if (and (= block.type :text) (= (type block.text) :string))
                          {:type :text :text block.text}
                          (and (= block.type :thinking) (= (type block.thinking) :string))
                          {:type :thinking :text block.thinking} nil)]
            (when value
              (table.insert fx (delta id value))
              (set state (misa.patch state {:saw_content true})))))
      (set state (misa.patch state {:messages {key {:final_blocks {slot true}}}})))
    {:state (misa.patch state {:messages {key {:finalized true}}}) : fx}))

(fn user-record [previous record id]
  (when (= (type record.message) :table)
    (var state previous)
    (local fx [])
    (each [_ block (ipairs (or record.message.content []))]
      (when (and (= block.type :tool_result) (not (. state.tool_results block.tool_use_id)))
        (local parts [])
        (if (= (type block.content) :string) (table.insert parts block.content)
            (each [_ item (ipairs (or block.content []))]
              (when (= item.type :text) (table.insert parts item.text))))
        (table.insert fx (emit id :agent/stream-tool-result
                              {:tool_call_id block.tool_use_id :is_error (= block.is_error true)
                               :text (table.concat parts "\n")}))
        (set state (misa.patch state {:tool_results {block.tool_use_id true}}))))
    {: state : fx}))

(fn result-record [state record id]
  (local fx [])
  (if record.is_error
      (table.insert fx (emit id :agent/stream-error {:message (tostring (or record.result "Claude request failed"))}))
      (and (not state.saw_content) (= (type record.result) :string))
      (table.insert fx (delta id {:type :text :text record.result})))
  (table.insert fx (usage id record.usage record.total_cost_usd))
  {:state (misa.patch state {:result true :failed (= record.is_error true)}) : fx :finish true})

(local records
       {:stream_event (fn [state record id]
                         (local streamed (and (= (type record.event) :table) record.event))
                         (local handler (and streamed (. partials streamed.type)))
                         (when handler (handler state streamed id)))
        :assistant assistant-record :user user-record :result result-record})

(fn stream-update [id state fx]
  {:patch {:providers {:claude_streams {id (misa.replace state)}}} : fx})

(fn stream [db event]
  (local state (or (and db.providers db.providers.claude_streams
                        (. db.providers.claude_streams event.id)) (fresh)))
  (if (= event.phase :start)
      (stream-update event.id (fresh) [(emit event.id :agent/stream-start)])
      (= event.phase :end)
      (let [next-event (if (not event.ok)
                          (emit event.id :agent/stream-error
                                {:message (or event.message (when (not= event.body "") event.body)
                                              (.. "claude exited " (tostring event.status)))})
                          (not state.result)
                          (emit event.id :agent/stream-error {:message "Claude returned no result record"})
                          (emit event.id :agent/stream-end))]
        (stream-update event.id nil (if state.failed [] [next-event])))
      state.result nil
      (do
        (var next state)
        (local fx [])
        (var finished false)
        (each [_ record (ipairs (or event.records [])) &until finished]
          (local problem (mcp-problem record))
          (local handler (. records record.type))
          (local result (if problem
                           {:state (misa.patch next {:result true :failed true}) :finish true
                            :fx [(emit event.id :agent/stream-error {:message problem})]}
                           (and handler (handler next record event.id))))
          (when result
            (set next (or result.state next))
            (each [_ effect (ipairs (or result.fx []))] (table.insert fx effect))
            (when result.finish
              (set finished true)
              (table.insert fx {:id event.id :type :operation/finish}))))
        (stream-update event.id next fx))))

{:setup (fn [context]
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/auth-provider
                         :value {:description "Claude Pro/Max via Claude Code"
                                 :id :claude
                                 :label :Claude
                                 :model_provider :claude
                                 :strategy :cli_handoff}})
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table) providers.claude)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local executable (or config.executable :claude))
          (assert (and (= (type executable) :string) (not= executable ""))
                  "config.providers.claude.executable must be nonempty")
          (local host (or context.host {}))
          (local mcp-command (or (or config.mcp_command host.executable) :misa))
          (local mcp-arguments (or config.mcp_arguments
                                   (or (and host.config_path
                                            [:mcp :--config host.config_path])
                                       [:mcp])))
          (assert (and (= (type mcp-command) :string) (not= mcp-command ""))
                  "config.providers.claude.mcp_command must be nonempty")
          (assert (= (type mcp-arguments) :table)
                  "config.providers.claude.mcp_arguments must be an array")
          (assert (or (= config.max_plan nil)
                      (= (type config.max_plan) :boolean))
                  "config.providers.claude.max_plan must be boolean")
          (local max-plan (= config.max_plan true))
          (local serializer-id :claude.cli)
          (table.insert setup-fx
                        {:type :register/request-options-serializer
                         :id serializer-id
                         :serializer {:accepts (fn [name]
                                                 (= name :reasoning_effort))
                                      :serialize (fn [argv name value]
                                                   (if (not= name
                                                             :reasoning_effort)
                                                       false
                                                       (do
                                                         (tset argv
                                                               (+ (length argv)
                                                                  1)
                                                               :--effort)
                                                         (tset argv
                                                               (+ (length argv)
                                                                  1)
                                                               value)
                                                         true)))}})
          (local reasoning-api
                 {:request_options {:reasoning_effort {:choices [:low
                                                                 :medium
                                                                 :high
                                                                 :max]
                                                       :default :high}}
                  :request_options_serializer serializer-id})
          (local configured-models
                 (or config.models
                     [{:context_window 1000000
                       :id :claude/claude-fable-5-1
                       :label "Claude Fable 5.1"
                       :model :claude-fable-5-1}
                      {:context_window (or (and max-plan 1000000) 200000)
                       :id :claude/claude-opus-5
                       :label "Claude Opus 5"
                       :model :claude-opus-5}
                      {:context_window 1000000
                       :id :claude/claude-sonnet-5
                       :label "Claude Sonnet 5"
                       :model :claude-sonnet-5}
                      {:context_window 200000
                       :id :claude/claude-haiku-4-5-20251001
                       :label "Claude Haiku 4.5"
                       :model :claude-haiku-4-5-20251001}]))
          (assert (and (= (type configured-models) :table)
                       (> (length configured-models) 0))
                  "config.providers.claude.models must be nonempty")
          (each [_ model (ipairs configured-models)]
            (assert (and (and (= (type model) :table)
                              (= (type model.id) :string))
                         (= (type model.model) :string))
                    "invalid Claude model")
            (local source (or model.api reasoning-api))
            (local api (if (and (= (type source) :table) (= (type source.request_options) :table))
                           (misa.patch source {:request_options_serializer serializer-id}) source))
            (table.insert setup-fx
                          {:type :register/model
                           :value {: api
                                   :context_window model.context_window
                                   :id model.id
                                   :label (or model.label model.id)
                                   :model model.model
                                   :provider :claude}}))
          (when (= config.max_plan nil)
            (table.insert setup-fx
                          {:type :register/event
                           :name :models/provider-availability
                           :handler (fn [db event]
                                      (if (or (or (not= event.provider :claude)
                                                  (= event.subscription_type
                                                     nil))
                                              (= event.subscription_type
                                                 misa.json_null))
                                          nil
                                          (do
                                            (local has-extended-opus
                                                   (or (or (= event.subscription_type
                                                              :max)
                                                           (= event.subscription_type
                                                              :team))
                                                       (= event.subscription_type
                                                          :enterprise)))
                                            (local updates {})
                                            (each [_ model (ipairs configured-models)]
                                              (when (model.model:match "^claude%-opus%-")
                                                (tset updates
                                                      (+ (length updates) 1)
                                                      {:context_window (or (and has-extended-opus
                                                                                1000000)
                                                                           200000)
                                                       :id model.id})))
                                            {:patch {:providers {:claude {:subscription_type event.subscription_type}}}
                                             :fx [{:event {:models updates
                                                           :provider :claude
                                                           :type :models/update}
                                                   :type :dispatch}]})))}))
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.claude
                         :handler (fn [effect]
                                    (local argv
                                           [executable
                                            :--print
                                            :--input-format
                                            :stream-json
                                            :--output-format
                                            :stream-json
                                            :--verbose
                                            :--include-partial-messages
                                            :--model
                                            effect.model
                                            :--tools
                                            ""
                                            :--strict-mcp-config
                                            :--permission-mode
                                            :dontAsk
                                            :--no-session-persistence])
                                    (misa.serialize_request_options serializer-id
                                                                    (or effect.request_options
                                                                        {})
                                                                    argv)
                                    (when (> (length effect.tools) 0)
                                      (local allowed {})
                                      (each [_ tool (ipairs effect.tools)]
                                        (tset allowed (+ (length allowed) 1)
                                              (.. :mcp__misa__ tool.name)))
                                      (tset argv (+ (length argv) 1)
                                            :--mcp-config)
                                      (tset argv (+ (length argv) 1)
                                            (mcp-config mcp-command
                                                        mcp-arguments))
                                      (tset argv (+ (length argv) 1)
                                            :--allowedTools)
                                      (tset argv (+ (length argv) 1)
                                            (table.concat allowed ",")))
                                    (when effect.system_prompt
                                      (tset argv (+ (length argv) 1)
                                            :--system-prompt)
                                      (tset argv (+ (length argv) 1)
                                            effect.system_prompt))
                                    {: argv
                                     :completion :provider/claude-complete
                                     :id effect.id
                                     :stdin_json {:message {:content (prompt-content effect.messages)
                                                            :role :user}
                                                  :parent_tool_use_id misa.json_null
                                                  :type :user}
                                     :stdout_format :json_lines_stream
                                     :type :process/run})})
          (table.insert setup-fx {:type :register/event :name :provider/claude-complete :handler stream})
          (each [name registry (pairs {:register/claude-record records :register/claude-stream-event partials})]
            (table.insert setup-fx
                          {:type :register/setup-effect : name
                           :handler (fn [effect]
                                      (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                   (= (type effect.value) :function) (not (. registry effect.id)))
                                              "invalid or duplicate Claude record handler")
                                      (tset registry effect.id effect.value))}))
          {:fx setup-fx})}
