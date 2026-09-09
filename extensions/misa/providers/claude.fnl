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

(fn fresh []
  {:message_seq 0
   :messages {}
   :tools {}
   :block_tools {}
   :tool_results {}
   :result false
   :saw_content false})

(fn message-key [state] (or state.current_message "seq:0"))

(fn block-key [state index] (.. (message-key state) ":" (tostring index)))

(fn body [state index value fx id]
  (let [key (message-key state)
        slot (tostring index)
        finalized (and (. state.messages key)
                       (. state.messages key :final_blocks)
                       (. state.messages key :final_blocks slot))]
    (if (and value (not= value.text "") (not finalized))
        (do
          (table.insert fx (delta id value))
          (misa.patch state {:messages {key {:streamed_blocks {slot true}}}
                             :saw_content true}))
        state)))

(fn tool-name [name]
  (if (= (type name) :string) (name:gsub "^mcp__misa__" "") name))

(fn tool [state index block complete fx id]
  (let [call-id (assert block.id "Claude tool call requires an id")
        previous (. state.tools call-id)
        key (or (and previous previous.index) (block-key state index))]
    (when (or (not previous) (and complete (not previous.complete)))
      (table.insert fx
                    (delta id
                           {:type :tool_call
                            :execution :provider
                            :index key
                            :id call-id
                            :name (tool-name block.name)
                            :arguments (when complete (or block.input {}))
                            :arguments_json (when (not complete) "")})))
    (misa.patch state
                {:saw_content true
                 :tools {call-id {:index key
                                  :complete (or complete
                                                (and previous previous.complete)
                                                false)}}
                 :block_tools {(block-key state index) call-id}})))

(fn usage [id raw cost]
  (let [value (if (= (type raw) :table) raw {})]
    (emit id :agent/stream-usage
          {:usage {:input_tokens (or value.input_tokens 0)
                   :output_tokens (or value.output_tokens 0)
                   :cache_read_tokens (or value.cache_read_input_tokens 0)
                   :cache_write_tokens (or value.cache_creation_input_tokens 0)
                   :input_includes_cache false
                   :cost_usd cost}})))

(fn message-start [state record id]
  (when (= (type record.message) :table)
    (let [sequence (+ state.message_seq 1)
          key (if record.message.id
                  (.. "id:" record.message.id)
                  (.. "seq:" sequence))]
      {:state (misa.patch state {:message_seq sequence :current_message key})
       :fx [(usage id record.message.usage)]})))

(fn message-delta [_ record id]
  {:fx [(emit id :agent/stream-usage
              {:stop_reason (and (= (type record.delta) :table)
                                 record.delta.stop_reason)
               :usage {:output_tokens (or (and (= (type record.usage) :table)
                                               record.usage.output_tokens)
                                          0)}})]})

(fn content-block-start [state record id]
  (when (= (type record.content_block) :table)
    (let [block record.content_block
          fx []
          value (if (and (= block.type :text) (= (type block.text) :string)
                         (not= block.text ""))
                    {:type :text :text block.text}
                    (and (= block.type :thinking)
                         (= (type block.thinking) :string)
                         (not= block.thinking ""))
                    {:type :thinking :text block.thinking}
                    nil)
          next (body state record.index value fx id)]
      {:state (if (= block.type :tool_use)
                  (tool next record.index block false fx id)
                  next)
       : fx})))

(fn content-block-delta [state record id]
  (when (= (type record.delta) :table)
    (let [value record.delta
          fx []
          content (if (and (= value.type :text_delta)
                           (= (type value.text) :string))
                      {:type :text :text value.text}
                      (and (= value.type :thinking_delta)
                           (= (type value.thinking) :string))
                      {:type :thinking :text value.thinking}
                      nil)
          next (body state record.index content fx id)]
      (when (= value.type :input_json_delta)
        (let [call-id (. state.block_tools (block-key state record.index))
              call (and call-id (. state.tools call-id))]
          (when (not (and call call.complete))
            (table.insert fx
                          (delta id
                                 {:type :tool_call
                                  :index (or (and call call.index)
                                             (block-key state record.index))
                                  :arguments_json_delta (or value.partial_json
                                                            "")})))))
      {:state (if (= value.type :input_json_delta)
                  (misa.patch next {:saw_content true})
                  next)
       : fx})))

(local partials {:message_start message-start
                 :message_delta message-delta
                 :content_block_start content-block-start
                 :content_block_delta content-block-delta})

(fn assistant-record [previous record id]
  (when (= (type record.message) :table)
    (let [current (message-key previous)
          advance (and (not record.message.id) (. previous.messages current)
                       (. previous.messages current :finalized))
          sequence (+ previous.message_seq (if advance 1 0))
          key (if record.message.id (.. "id:" record.message.id)
                  advance (.. "seq:" sequence)
                  current)]
      (var state
           (misa.patch previous {:current_message key :message_seq sequence}))
      (let [fx []]
        (each [index block (ipairs (or record.message.content []))]
          (let [slot (tostring (- index 1))
                entry (or (. state.messages key) {})]
            (if (= block.type :tool_use)
                (set state (tool state (- index 1) block true fx id))
                (and (not (and entry.streamed_blocks
                               (. entry.streamed_blocks slot)))
                     (not (and entry.final_blocks (. entry.final_blocks slot))))
                (let [value (if (and (= block.type :text)
                                     (= (type block.text) :string))
                                {:type :text :text block.text}
                                (and (= block.type :thinking)
                                     (= (type block.thinking) :string))
                                {:type :thinking :text block.thinking}
                                nil)]
                  (when value
                    (table.insert fx (delta id value))
                    (set state (misa.patch state {:saw_content true})))))
            (set state
                 (misa.patch state
                             {:messages {key {:final_blocks {slot true}}}}))))
        {:state (misa.patch state {:messages {key {:finalized true}}}) : fx}))))

(fn user-record [previous record id]
  (when (= (type record.message) :table)
    (var state previous)
    (let [fx []]
      (each [_ block (ipairs (or record.message.content []))]
        (when (and (= block.type :tool_result)
                   (not (. state.tool_results block.tool_use_id)))
          (let [parts []]
            (if (= (type block.content) :string)
                (table.insert parts block.content)
                (each [_ item (ipairs (or block.content []))]
                  (when (= item.type :text) (table.insert parts item.text))))
            (table.insert fx
                          (emit id :agent/stream-tool-result
                                {:tool_call_id block.tool_use_id
                                 :is_error (= block.is_error true)
                                 :text (table.concat parts "\n")}))
            (set state
                 (misa.patch state {:tool_results {block.tool_use_id true}})))))
      {: state : fx})))

(fn result-record [state record id]
  (let [fx []]
    (if record.is_error
        (table.insert fx
                      (emit id :agent/stream-error
                            {:message (tostring (or record.result
                                                    "Claude request failed"))}))
        (and (not state.saw_content) (= (type record.result) :string))
        (table.insert fx (delta id {:type :text :text record.result})))
    (table.insert fx (usage id record.usage record.total_cost_usd))
    {:state (misa.patch state {:result true :failed (= record.is_error true)})
     : fx
     :finish true}))

(fn quota-number [value]
  (when (and (= (type value) :number) (= value value) (>= value 0)
             (< value math.huge))
    value))

(local quota-labels {:five_hour "Claude · 5h"
                     :seven_day "Claude · 7d"
                     :seven_day_oauth_apps "Claude OAuth apps · 7d"
                     :seven_day_opus "Claude Opus · 7d"
                     :seven_day_sonnet "Claude Sonnet · 7d"
                     :overage "Claude overage"})

(fn quota-window [id label value]
  (let [amount (quota-number value.utilization)
        used (when (and amount (<= amount 100)) amount)]
    {: id
     : label
     :source :cli
     :unit :percent
     : used
     :limit (when used 100)
     :remaining (when used (- 100 used))
     :reset_at (when (= (type value.resets_at) :string) value.resets_at)}))

(fn quota-snapshot [data]
  "Normalize Claude account limits into a quota snapshot."
  (let [windows []]
    (var extra-usage nil)
    (let [limits (and (= data.rate_limits_available true)
                      (= (type data.rate_limits) :table) data.rate_limits)]
      (when limits
        (let [keys (icollect [key value (pairs limits)]
                     (when (and (= (type key) :string) (not= key :extra_usage)
                                (= (type value) :table)
                                (or (not= value.utilization nil)
                                    (not= value.resets_at nil)))
                       key))]
          (table.sort keys)
          (each [_ key (ipairs keys)]
            (table.insert windows
                          (quota-window key (or (. quota-labels key) key)
                                        (. limits key))))
          (when (= (type limits.model_scoped) :table)
            (each [index value (ipairs limits.model_scoped)]
              (when (and (= (type value) :table)
                         (= (type value.display_name) :string))
                (table.insert windows
                              (quota-window (.. :model/ index "/"
                                                value.display_name)
                                            (.. value.display_name " · 7d")
                                            value)))))
          (let [extra limits.extra_usage]
            (when (and (= (type extra) :table)
                       (= (type extra.is_enabled) :boolean))
              (let [currency (if (= (type extra.currency) :string)
                                 (string.upper extra.currency)
                                 :USD)
                    decimals (quota-number extra.decimal_places)
                    scaled (and decimals (<= decimals 9) (= (% decimals 1) 0))
                    ;; Match the CLI's currency minor-unit conversion when decimal_places is absent.
                    divisor (if scaled (^ 10 decimals)
                                (or (= currency :JPY) (= currency :KRW)
                                    (= currency :VND)) 1
                                100)
                    used (quota-number extra.used_credits)
                    limit (quota-number extra.monthly_limit)]
                ;; CLI get_usage has no mutation control. Its settings page owns enablement
                ;; and monthly limits until a supported authenticated update adapter exists.
                (set extra-usage
                     {:enabled extra.is_enabled
                      : currency
                      :unit currency
                      :used (when used (/ used divisor))
                      :limit (when limit (/ limit divisor))
                      :unlimited (or (= extra.monthly_limit nil)
                                     (= extra.monthly_limit misa.json-null))
                      :manage_url "https://claude.ai/settings/usage"}))))))
      (let [available (accumulate [found false _ window (ipairs windows)]
                        (or found (not= window.used nil)))]
        {:source :cli
         : windows
         :extra_usage extra-usage
         :unavailable (not available)}))))

(fn usage-response [event]
  (when (and event.ok (= (type event.data) :table))
    (accumulate [found nil _ record (ipairs event.data)]
      (or found (let [response (and (= (type record) :table) record.response)]
                  (when (and (= (type record) :table)
                             (= record.type :control_response)
                             (= (type response) :table)
                             (= response.request_id event.id)
                             (= response.subtype :success)
                             (= (type response.response) :table))
                    response.response))))))

(fn merge-quota [previous incoming]
  (let [update (and incoming.windows (. incoming.windows 1))]
    (if (or (not previous) (not update) (not update.id)) incoming
        (let [windows []]
          (var found false)
          (each [_ window (ipairs (or previous.windows []))]
            (if (= window.id update.id)
                (do
                  (table.insert windows update)
                  (set found true))
                (table.insert windows window)))
          (when (not found) (table.insert windows update))
          (let [available (accumulate [known false _ window (ipairs windows)]
                            (or known (not= window.used nil)))]
            (misa.patch previous
                        {:windows (misa.replace windows)
                         :partial true
                         :source (if (= previous.source :stream) :stream
                                     :cli_stream)
                         :unavailable (not available)}))))))

(fn quota-record [_ record]
  (let [info record.rate_limit_info]
    (when (= (type info) :table)
      (let [fraction (quota-number info.utilization)
            used (when (and fraction (<= fraction 1)) (* fraction 100))
            kind (when (= (type info.rateLimitType) :string) info.rateLimitType)
            status (when (= (type info.status) :string) info.status)
            reset (quota-number info.resetsAt)]
        ;; A record is the latest affected window, not a complete account snapshot.
        ;; Do not retain older percentages when a new report omits utilization.
        {:fx [{:type :dispatch
               :event {:type :provider/claude-quota
                       :usage {:source :stream
                               :unavailable (= used nil)
                               :windows [{:id kind
                                          :source :stream
                                          :label (or (. quota-labels
                                                        (or kind ""))
                                                     kind "Claude")
                                          :unit :percent
                                          :status status
                                          :reset_at_unix reset
                                          :used used
                                          :limit (when used 100)
                                          :remaining (when used
                                                       (math.max 0 (- 100 used)))}]}}}]}))))

(fn stream-event [state record id]
  (let [streamed (and (= (type record.event) :table) record.event)
        handler (and streamed (. (misa.catalog :claude-stream-events)
                                 streamed.type))]
    (when handler (handler state streamed id))))

(local records {:stream_event stream-event
                :assistant assistant-record
                :user user-record
                :result result-record
                :rate_limit_event quota-record})

(fn stream-update [id state fx]
  {:patch {:providers {:claude_streams {id (misa.replace state)}}}
   :fx (misa.stream.effects fx)})

(fn stream [db event]
  "Translate streamed transport records into agent events."
  (let [state (or (and db.providers db.providers.claude_streams
                       (. db.providers.claude_streams event.id))
                  (fresh))]
    (if (= event.phase :start)
        (stream-update event.id (fresh) [(emit event.id :agent/stream-start)])
        (= event.phase :end)
        (let [next-event (if (not event.ok)
                             (emit event.id :agent/stream-error
                                   {:message (or event.message
                                                 (when (not= event.body "")
                                                   event.body)
                                                 (.. "claude exited "
                                                     (tostring event.status)))})
                             (not state.result)
                             (emit event.id :agent/stream-error
                                   {:message "Claude returned no result record"})
                             (emit event.id :agent/stream-end))]
          (stream-update event.id nil (if state.failed [] [next-event])))
        state.result
        nil
        (do
          (var next state)
          (let [fx []]
            (var finished false)
            (each [_ record (ipairs (or event.records [])) &until finished]
              (let [problem (mcp-problem record)
                    handler (. (misa.catalog :claude-records) record.type)
                    result (if problem
                               {:state (misa.patch next
                                                   {:result true :failed true})
                                :finish true
                                :fx [(emit event.id :agent/stream-error
                                           {:message problem})]}
                               (and handler (handler next record event.id)))]
                (when result
                  (set next (or result.state next))
                  (each [_ effect (ipairs (or result.fx []))]
                    (table.insert fx effect))
                  (when result.finish
                    (set finished true)
                    (table.insert fx {:id event.id :type :operation/finish})))))
            (stream-update event.id next fx))))))

(fn refresh-usage [config executable db event]
  "Describe a quota refresh request."
  (when (or (not event.provider) (= event.provider :claude))
    (let [provider (and db.providers db.providers.claude)]
      (if (and provider provider.usage_request)
          {:patch {:providers {:claude {:usage_again true}}}}
          (let [sequence (+ (or (and provider provider.usage_sequence) 0) 1)
                id (.. :claude-usage- sequence)]
            {:patch {:providers {:claude {:usage_sequence sequence
                                          :usage_request id}}}
             :fx [{:type :provider/process
                   : id
                   :completion :provider/claude-usage
                   :argv [executable
                          :--print
                          :--input-format
                          :stream-json
                          :--output-format
                          :stream-json
                          :--verbose
                          :--no-session-persistence
                          :--setting-sources
                          ""
                          :--settings
                          "{\"disableAllHooks\":true}"
                          :--strict-mcp-config
                          :--mcp-config
                          "{\"mcpServers\":{}}"]
                   :stdin_json {:type :control_request
                                :request_id id
                                :request {:subtype :get_usage}}
                   :stdout_format :json_lines
                   :timeouts (or config.usage_timeouts
                                 {:startup_ms 10000
                                  :idle_ms 10000
                                  :overall_ms 30000})}]})))))

(fn receive-usage [db event]
  "Apply a completed quota request."
  (let [provider (and db.providers db.providers.claude)]
    (when (and provider provider.usage_request
               (= provider.usage_request event.id))
      (let [data (usage-response event)
            snapshot (misa.patch (quota-snapshot (or data {}))
                                 {:summary (when (not data)
                                             "Claude CLI usage unavailable")})
            plan (and data (= (type data.subscription_type) :string)
                      data.subscription_type)
            updated {:type :dispatch :event {:type :usage/updated}}]
        {:patch {:providers {:claude {:usage_request misa.delete
                                      :usage_again misa.delete
                                      :subscription_type (when data
                                                           (misa.replace plan))
                                      :usage (misa.replace snapshot)}}}
         :fx (if provider.usage_again
                 [updated
                  {:type :dispatch
                   :event {:type :usage/refresh :provider :claude}}]
                 [updated])}))))

(fn provider-availability [configured-models db event]
  "Update model limits from account availability."
  (if (or (not= event.provider :claude) (= event.subscription_type nil)
          (= event.subscription_type misa.json-null))
      nil
      (let [has-extended-opus (or (= event.subscription_type :max)
                                  (= event.subscription_type :team)
                                  (= event.subscription_type :enterprise))
            updates {}]
        (each [_ model (ipairs configured-models)]
          (when (model.model:match "^claude%-opus%-")
            (tset updates (+ (length updates) 1)
                  {:context_window (or (and has-extended-opus 1000000) 200000)
                   :id model.id})))
        {:patch {:providers {:claude {:subscription_type event.subscription_type}}}
         :fx [{:event {:models updates :provider :claude :type :models/update}
               :type :dispatch}]})))

(fn request [config serializer-id executable effect cofx]
  "Build a Claude CLI request with explicit host and provider settings."
  (assert (and (= (type executable) :string) (not= executable ""))
          "Claude executable must be nonempty")
  (assert (or (= config.mcp_command nil)
              (and (= (type config.mcp_command) :string)
                   (not= config.mcp_command "")))
          "Claude MCP command must be nonempty")
  (assert (or (= config.mcp_arguments nil)
              (= (type config.mcp_arguments) :table))
          "Claude MCP arguments must be an array")
  (let [argv [executable
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
              :--no-session-persistence]]
    (each [_ argument (ipairs (misa.request-options.serialize serializer-id
                                                              (or effect.request_options
                                                                  {})))]
      (table.insert argv argument))
    (when (> (length effect.tools) 0)
      (let [host (or cofx.host {})
            mcp-command (or config.mcp_command host.executable :misa)
            mcp-arguments (or config.mcp_arguments
                              (and host.config_path
                                   [:mcp :--config host.config_path])
                              [:mcp])
            allowed {}]
        (each [_ tool (ipairs effect.tools)]
          (tset allowed (+ (length allowed) 1) (.. :mcp__misa__ tool.name)))
        (tset argv (+ (length argv) 1) :--mcp-config)
        (tset argv (+ (length argv) 1) (mcp-config mcp-command mcp-arguments))
        (tset argv (+ (length argv) 1) :--allowedTools)
        (tset argv (+ (length argv) 1) (table.concat allowed ","))))
    (when effect.system_prompt
      (tset argv (+ (length argv) 1) :--system-prompt)
      (tset argv (+ (length argv) 1) effect.system_prompt))
    {: argv
     :completion :provider/claude-complete
     :id effect.id
     :stdin_json {:message {:content (prompt-content effect.messages)
                            :role :user}
                  :parent_tool_use_id misa.json-null
                  :type :user}
     :stdout_format :json_lines_stream
     :type :provider/process}))

(fn receive-quota [db event]
  "Apply quota observations from the provider."
  (let [previous (and db.providers db.providers.claude
                      db.providers.claude.usage)]
    {:patch {:providers {:claude {:usage (misa.replace (merge-quota previous
                                                                    event.usage))}}}
     :fx [{:type :dispatch :event {:type :usage/updated}}]}))

{:partials partials
 :records records
 :stream stream
 :refresh-usage refresh-usage
 :receive-usage receive-usage
 :provider-availability provider-availability
 :request request
 :receive-quota receive-quota
 :quota-snapshot quota-snapshot}
