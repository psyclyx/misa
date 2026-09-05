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
            (var api (or model.api reasoning-api))
            (when (and (= (type api) :table)
                       (= (type api.request_options) :table))
              (local copy {})
              (each [key value (pairs api)] (tset copy key value))
              (set copy.request_options_serializer serializer-id)
              (set api copy))
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
                                            (set db.providers
                                                 (or db.providers {}))
                                            (set db.providers.claude
                                                 (or db.providers.claude {}))
                                            (set db.providers.claude.subscription_type
                                                 event.subscription_type)
                                            {: db
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
          (table.insert setup-fx
                        {:type :register/event
                         :name :provider/claude-complete
                         :handler (fn [db event]
                                    (set db.providers (or db.providers {}))
                                    (set db.providers.claude_streams
                                         (or db.providers.claude_streams {}))
                                    (if (= event.phase :start)
                                        (do
                                          (tset db.providers.claude_streams
                                                event.id
                                                {:message_seq 0
                                                 :result false
                                                 :saw_content false
                                                 :saw_stream_event false})
                                          {: db
                                           :fx [{:event {:id event.id
                                                         :type :agent/stream-start}
                                                 :type :dispatch}]})
                                        (do
                                          (local state
                                                 (or (. db.providers.claude_streams
                                                        event.id)
                                                     {:message_seq 0
                                                      :result false
                                                      :saw_content false
                                                      :saw_stream_event false}))
                                          (if (= event.phase :end)
                                              (do
                                                (tset db.providers.claude_streams
                                                      event.id nil)
                                                (var next-event nil)
                                                (if (not event.ok)
                                                    (set next-event
                                                         {:id event.id
                                                          :message (or event.message
                                                                       (or (and (not= event.body
                                                                                      "")
                                                                                event.body)
                                                                           (.. "claude exited "
                                                                               (tostring event.status))))
                                                          :type :agent/stream-error})
                                                    (not state.result)
                                                    (set next-event
                                                         {:id event.id
                                                          :message "Claude returned no result record"
                                                          :type :agent/stream-error})
                                                    (set next-event
                                                         {:id event.id
                                                          :type :agent/stream-end}))
                                                {: db
                                                 :fx [{:event next-event
                                                       :type :dispatch}]})
                                              (do
                                                (var (fx terminal)
                                                     (values {} false))
                                                (each [_ record (ipairs (or event.records
                                                                            {}))]
                                                  (local ___partial___
                                                         (or (and (= record.type
                                                                     :stream_event)
                                                                  record.event)
                                                             nil))
                                                  (when (and (= (type ___partial___)
                                                                :table)
                                                             (or (= ___partial___.type
                                                                    :content_block_start)
                                                                 (= ___partial___.type
                                                                    :content_block_delta)))
                                                    (set state.saw_stream_event
                                                         true)
                                                    (lua :break)))
                                                (each [_ record (ipairs (or event.records
                                                                            {}))]
                                                  (local problem
                                                         (mcp-problem record))
                                                  (when problem
                                                    (set (state.result terminal)
                                                         (values true true))
                                                    (table.insert fx
                                                                  {:type :dispatch
                                                                   :event {:type :agent/stream-error
                                                                           :id event.id
                                                                           :message problem}})
                                                    (lua :break))
                                                  (if (and (= record.type
                                                              :stream_event)
                                                           (= (type record.event)
                                                              :table))
                                                      (do
                                                        (local ___partial___
                                                               record.event)
                                                        (if (and (= ___partial___.type
                                                                    :content_block_start)
                                                                 (= (type ___partial___.content_block)
                                                                    :table))
                                                            (do
                                                              (local block
                                                                     ___partial___.content_block)
                                                              (if (= block.type
                                                                     :tool_use)
                                                                  (do
                                                                    (set state.saw_content
                                                                         true)
                                                                    (tset fx
                                                                          (+ (length fx)
                                                                             1)
                                                                          {:event {:delta {:arguments_json ""
                                                                                           :execution :provider
                                                                                           :id block.id
                                                                                           :index (.. (tostring state.message_seq)
                                                                                                      ":"
                                                                                                      (tostring ___partial___.index))
                                                                                           :name block.name
                                                                                           :type :tool_call}
                                                                                   :id event.id
                                                                                   :type :agent/stream-delta}
                                                                           :type :dispatch}))
                                                                  (and (and (= block.type
                                                                               :text)
                                                                            (= (type block.text)
                                                                               :string))
                                                                       (not= block.text
                                                                             ""))
                                                                  (do
                                                                    (set state.saw_content
                                                                         true)
                                                                    (tset fx
                                                                          (+ (length fx)
                                                                             1)
                                                                          {:event {:delta {:text block.text
                                                                                           :type :text}
                                                                                   :id event.id
                                                                                   :type :agent/stream-delta}
                                                                           :type :dispatch}))
                                                                  (and (and (= block.type
                                                                               :thinking)
                                                                            (= (type block.thinking)
                                                                               :string))
                                                                       (not= block.thinking
                                                                             ""))
                                                                  (do
                                                                    (set state.saw_content
                                                                         true)
                                                                    (tset fx
                                                                          (+ (length fx)
                                                                             1)
                                                                          {:event {:delta {:text block.thinking
                                                                                           :type :thinking}
                                                                                   :id event.id
                                                                                   :type :agent/stream-delta}
                                                                           :type :dispatch}))))
                                                            (and (= ___partial___.type
                                                                    :content_block_delta)
                                                                 (= (type ___partial___.delta)
                                                                    :table))
                                                            (do
                                                              (local delta
                                                                     ___partial___.delta)
                                                              (if (and (= delta.type
                                                                          :text_delta)
                                                                       (= (type delta.text)
                                                                          :string))
                                                                  (do
                                                                    (set state.saw_content
                                                                         true)
                                                                    (tset fx
                                                                          (+ (length fx)
                                                                             1)
                                                                          {:event {:delta {:text delta.text
                                                                                           :type :text}
                                                                                   :id event.id
                                                                                   :type :agent/stream-delta}
                                                                           :type :dispatch}))
                                                                  (and (= delta.type
                                                                          :thinking_delta)
                                                                       (= (type delta.thinking)
                                                                          :string))
                                                                  (do
                                                                    (set state.saw_content
                                                                         true)
                                                                    (tset fx
                                                                          (+ (length fx)
                                                                             1)
                                                                          {:event {:delta {:text delta.thinking
                                                                                           :type :thinking}
                                                                                   :id event.id
                                                                                   :type :agent/stream-delta}
                                                                           :type :dispatch}))
                                                                  (= delta.type
                                                                     :input_json_delta)
                                                                  (do
                                                                    (set state.saw_content
                                                                         true)
                                                                    (tset fx
                                                                          (+ (length fx)
                                                                             1)
                                                                          {:event {:delta {:arguments_json_delta (or delta.partial_json
                                                                                                                     "")
                                                                                           :index (.. (tostring state.message_seq)
                                                                                                      ":"
                                                                                                      (tostring ___partial___.index))
                                                                                           :type :tool_call}
                                                                                   :id event.id
                                                                                   :type :agent/stream-delta}
                                                                           :type :dispatch}))))
                                                            (and (= ___partial___.type
                                                                    :message_start)
                                                                 (= (type ___partial___.message)
                                                                    :table))
                                                            (do
                                                              (set state.message_seq
                                                                   (+ state.message_seq
                                                                      1))
                                                              (local usage
                                                                     (or (and (= (type ___partial___.message.usage)
                                                                                 :table)
                                                                              ___partial___.message.usage)
                                                                         {}))
                                                              (tset fx
                                                                    (+ (length fx)
                                                                       1)
                                                                    {:event {:id event.id
                                                                             :type :agent/stream-usage
                                                                             :usage {:cache_read_tokens (or usage.cache_read_input_tokens
                                                                                                            0)
                                                                                     :cache_write_tokens (or usage.cache_creation_input_tokens
                                                                                                             0)
                                                                                     :input_includes_cache false
                                                                                     :input_tokens (or usage.input_tokens
                                                                                                       0)
                                                                                     :output_tokens (or usage.output_tokens
                                                                                                        0)}}
                                                                     :type :dispatch}))
                                                            (= ___partial___.type
                                                               :message_delta)
                                                            (do
                                                              (local usage
                                                                     (or (and (= (type ___partial___.usage)
                                                                                 :table)
                                                                              ___partial___.usage)
                                                                         {}))
                                                              (tset fx
                                                                    (+ (length fx)
                                                                       1)
                                                                    {:event {:id event.id
                                                                             :stop_reason (or (and (= (type ___partial___.delta)
                                                                                                      :table)
                                                                                                   ___partial___.delta.stop_reason)
                                                                                              nil)
                                                                             :type :agent/stream-usage
                                                                             :usage {:output_tokens (or usage.output_tokens
                                                                                                        0)}}
                                                                     :type :dispatch}))))
                                                      (and (and (= record.type
                                                                   :assistant)
                                                                (= (type record.message)
                                                                   :table))
                                                           (not state.saw_stream_event))
                                                      (each [_ block (ipairs (or record.message.content
                                                                                 {}))]
                                                        (if (and (= block.type
                                                                    :text)
                                                                 (= (type block.text)
                                                                    :string))
                                                            (do
                                                              (set state.saw_content
                                                                   true)
                                                              (tset fx
                                                                    (+ (length fx)
                                                                       1)
                                                                    {:event {:delta {:text block.text
                                                                                     :type :text}
                                                                             :id event.id
                                                                             :type :agent/stream-delta}
                                                                     :type :dispatch}))
                                                            (and (= block.type
                                                                    :thinking)
                                                                 (= (type block.thinking)
                                                                    :string))
                                                            (do
                                                              (set state.saw_content
                                                                   true)
                                                              (tset fx
                                                                    (+ (length fx)
                                                                       1)
                                                                    {:event {:delta {:text block.thinking
                                                                                     :type :thinking}
                                                                             :id event.id
                                                                             :type :agent/stream-delta}
                                                                     :type :dispatch}))
                                                            (= block.type
                                                               :tool_use)
                                                            (do
                                                              (set state.saw_content
                                                                   true)
                                                              (tset fx
                                                                    (+ (length fx)
                                                                       1)
                                                                    {:event {:delta {:arguments block.input
                                                                                     :execution :provider
                                                                                     :id block.id
                                                                                     :name block.name
                                                                                     :type :tool_call}
                                                                             :id event.id
                                                                             :type :agent/stream-delta}
                                                                     :type :dispatch}))))
                                                      (and (= record.type :user)
                                                           (= (type record.message)
                                                              :table))
                                                      (each [_ block (ipairs (or record.message.content
                                                                                 {}))]
                                                        (when (= block.type
                                                                 :tool_result)
                                                          (local parts {})
                                                          (if (= (type block.content)
                                                                 :string)
                                                              (tset parts 1
                                                                    block.content)
                                                              (each [_ item (ipairs (or block.content
                                                                                        {}))]
                                                                (when (= item.type
                                                                         :text)
                                                                  (tset parts
                                                                        (+ (length parts)
                                                                           1)
                                                                        item.text))))
                                                          (tset fx
                                                                (+ (length fx)
                                                                   1)
                                                                {:event {:id event.id
                                                                         :is_error (= block.is_error
                                                                                      true)
                                                                         :text (table.concat parts
                                                                                             "\n")
                                                                         :tool_call_id block.tool_use_id
                                                                         :type :agent/stream-tool-result}
                                                                 :type :dispatch})))
                                                      (= record.type :result)
                                                      (do
                                                        (set (state.result terminal)
                                                             (values true true))
                                                        (if record.is_error
                                                            (tset fx
                                                                  (+ (length fx)
                                                                     1)
                                                                  {:event {:id event.id
                                                                           :message (tostring (or record.result
                                                                                                  "Claude request failed"))
                                                                           :type :agent/stream-error}
                                                                   :type :dispatch})
                                                            (and (not state.saw_content)
                                                                 (= (type record.result)
                                                                    :string))
                                                            (tset fx
                                                                  (+ (length fx)
                                                                     1)
                                                                  {:event {:delta {:text record.result
                                                                                   :type :text}
                                                                           :id event.id
                                                                           :type :agent/stream-delta}
                                                                   :type :dispatch}))
                                                        (local usage
                                                               (or (and (= (type record.usage)
                                                                           :table)
                                                                        record.usage)
                                                                   {}))
                                                        (tset fx
                                                              (+ (length fx) 1)
                                                              {:event {:id event.id
                                                                       :type :agent/stream-usage
                                                                       :usage {:cache_read_tokens (or usage.cache_read_input_tokens
                                                                                                      0)
                                                                               :cache_write_tokens (or usage.cache_creation_input_tokens
                                                                                                       0)
                                                                               :cost_usd record.total_cost_usd
                                                                               :input_includes_cache false
                                                                               :input_tokens (or usage.input_tokens
                                                                                                 0)
                                                                               :output_tokens (or usage.output_tokens
                                                                                                  0)}}
                                                               :type :dispatch})
                                                        (lua :break))))
                                                (tset db.providers.claude_streams
                                                      event.id state)
                                                (when terminal
                                                  (tset fx (+ (length fx) 1)
                                                        {:id event.id
                                                         :type :operation/finish}))
                                                {: db : fx})))))})
          {:fx setup-fx})}
