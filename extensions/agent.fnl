;; Provider-neutral conversation and tool orchestration.

(fn content [value label]
  (assert (= (type value) :table) (.. label " content must be an array"))
  (each [_ block (ipairs value)]
    (assert (and (= (type block) :table) (= (type block.type) :string))
            (.. label " content block is invalid"))
    (if (or (= block.type :text) (= block.type :thinking))
        (assert (= (type block.text) :string)
                (.. block.type " content must carry text"))
        (= block.type :tool_call)
        (do
          (assert (and (= (type block.id) :string) (not= block.id ""))
                  "tool call id must be nonempty")
          (assert (and (= (type block.name) :string) (not= block.name ""))
                  "tool call name must be nonempty")
          (assert (= (type block.arguments) :table)
                  "tool call arguments must be a table")
          (assert (= block.arguments_json nil)
                  "provider JSON must not enter canonical history"))
        (error (.. "unsupported assistant content block: " block.type))))
  value)

(fn selected-model [db]
  (let [state (assert db.models "model state is not initialized")]
    (each [_ model (ipairs (or state.entries {}))]
      (when (= model.id state.selected) (lua "return model")))
    nil))

(fn appended [items value]
  (local next (icollect [_ item (ipairs (or items []))] item))
  (table.insert next value)
  next)

(fn request [db agent]
  (local selected (assert (selected-model db) "selected model became unavailable"))
  (var (options problem) (values {} nil))
  (when misa.prepare_request_options
    (set (options problem) (misa.prepare_request_options db selected)))
  (if problem (values agent nil problem)
      (let [sequence (+ agent.request_seq 1)
            id (.. :agent- sequence)]
        (values (misa.patch agent {:request_seq sequence :active_request_id id :status :working
                                   :cancel_requested false :request_model selected.id})
                {: id :messages agent.messages :model selected.model :request_options options
                 :system_prompt agent.system_prompt :tools (misa.tools)
                 :type (.. :provider. selected.provider)}))))

(fn blocked [agent problem]
  (values (misa.patch agent {:status :ready :active_request_id misa.delete})
          [{:event {:level :error : problem :text problem.message :type :transcript/harness} :type :dispatch}
           {:event {:status :ready :type :agent/status} :type :dispatch}
           {:event {:exit agent.exit_after_response :type :agent/completed} :type :dispatch}]))

(fn record-usage [agent usage]
  (if (= usage nil) agent
      (do
        (assert (= (type usage) :table) "usage must be a table")
        (local normalized {})
        (local totals {})
        (each [_ name (ipairs [:input_tokens :output_tokens :cache_read_tokens :cache_write_tokens])]
          (local value (or (. usage name) 0))
          (assert (and (= (type value) :number) (>= value 0) (= (% value 1) 0))
                  (.. name " must be a nonnegative integer"))
          (tset normalized name value)
          (tset totals name (+ (. agent.usage name) value)))
        (tset normalized :input_includes_cache usage.input_includes_cache)
        (when (and (= (type usage.cost_usd) :number) (>= usage.cost_usd 0))
          (tset normalized :cost_usd usage.cost_usd))
        (misa.patch agent {:usage totals :last_usage (misa.replace normalized)}))))

(fn tool-result [call-id text is-error]
  {:content [{:text (tostring text) :type :text}]
   :is_error (= is-error true)
   :role :tool
   :tool_call_id call-id})

(fn flush-tool-results [agent]
  (local batch (assert agent.tool_batch "tool result batch is missing"))
  (local messages (icollect [_ message (ipairs agent.messages)] message))
  (each [_ call-id (ipairs batch.order)]
    (local result (assert (. batch.results call-id) (.. "tool result is missing: " call-id)))
    (table.insert messages result))
  (misa.patch agent {:messages (misa.replace messages) :tool_batch misa.delete}))

(fn normalize-tool-arguments [blocks]
  (local failures {})
  (local normalized
         (icollect [_ block (ipairs blocks)]
           (if (not= block.type :tool_call) block
               (let [arguments (if block.arguments block.arguments
                                   (do
                                     (assert (= (type block.arguments_json) :string) "tool call arguments are missing")
                                     (assert (and misa.json (= (type misa.json.decode) :function))
                                             "JSON extension is required for provider tool calls")
                                     (local (ok decoded) (pcall misa.json.decode (if (= block.arguments_json "") "{}" block.arguments_json)))
                                     (if (and ok (= (type decoded) :table)) decoded
                                         (do
                                           (tset failures block.id (if ok "tool arguments must be a JSON object" (tostring decoded)))
                                           {}))))]
                 (misa.patch block {:arguments (misa.replace arguments) :arguments_json misa.delete})))))
  (values normalized failures))

(fn complete-response [db previous id raw-blocks usage stop-reason]
  (local (blocks argument-failures) (normalize-tool-arguments raw-blocks))
  (content blocks :assistant)
  (let [stream previous.stream]
    (local canonical (icollect [_ block (ipairs blocks)]
                       (misa.patch block {:transcript_id misa.delete :execution misa.delete})))
    (var agent (misa.patch (record-usage previous usage)
                           {:messages (misa.replace (appended previous.messages
                                                              {:content canonical :provider_state (and stream stream.provider_state)
                                                               :role :assistant}))
                            :active_request_id misa.delete :accepted_request_id id :stream misa.delete
                            :tool_batch misa.delete}))
    (var (effects saw-tool) (values {} false))
    (if stream (each [_ block (ipairs blocks)]
                 (tset effects (+ (length effects) 1)
                       {:event {:arguments block.arguments
                                :block_id block.transcript_id
                                :call_id block.id
                                :name block.name
                                :response_id id
                                :type :transcript/block-end}
                        :type :dispatch}))
        (do
          (tset effects (+ (length effects) 1)
                {:event {:model agent.request_model
                         :response_id id
                         :role :assistant
                         :type :transcript/response-start}
                 :type :dispatch})
          (each [index block (ipairs blocks)]
            (local block-id (.. id "/" index))
            (tset effects (+ (length effects) 1)
                  {:event {:block_id block-id
                           :call_id block.id
                           :kind (or (and (= block.type :text) :assistant)
                                     block.type)
                           :name block.name
                           :response_id id
                           :type :transcript/block-start}
                   :type :dispatch})
            (tset effects (+ (length effects) 1)
                  {:event {:arguments block.arguments
                           :block_id block-id
                           :response_id id
                           :text block.text
                           :type :transcript/block-delta}
                   :type :dispatch})
            (tset effects (+ (length effects) 1)
                  {:event {:block_id block-id
                           :response_id id
                           :type :transcript/block-end}
                   :type :dispatch}))))
    (tset effects (+ (length effects) 1)
          {:event {:cost_usd (or (and usage usage.cost_usd)
                                 (and (and stream stream.usage)
                                      stream.usage.cost_usd))
                   :model agent.request_model
                   :response_id id
                   :type :transcript/response-end
                   :usage (or usage (and stream stream.usage))}
           :type :dispatch})
    (tset effects (+ (length effects) 1)
          {:event {:last_usage agent.last_usage
                   :type :agent/usage
                   :usage agent.usage}
           :type :dispatch})
    (each [_ block (ipairs blocks)]
      (when (= block.type :tool_call)
        (if (= block.execution :provider)
            (do
              (var result
                   (and (and stream stream.tool_results)
                        (. stream.tool_results block.id)))
              (set result (or result
                              (tool-result block.id
                                           "Provider did not report a tool result"
                                           true)))
              (set agent (misa.patch agent {:messages (misa.replace (appended agent.messages result))}))
              (tset effects (+ (length effects) 1)
                    {:event {:id block.id
                             :is_error result.is_error
                             :text (. result.content 1 :text)
                             :type :transcript/tool-result}
                     :type :dispatch}))
            (do
              (set saw-tool true)
              (when (not agent.tool_batch)
                (set agent (misa.patch agent {:tool_batch (misa.replace {:order [] :results {}})})))
              (assert (not (. agent.tool_batch.results block.id))
                      "duplicate tool call id")
              (set agent (misa.patch agent {:tool_batch {:order (misa.replace (appended agent.tool_batch.order block.id))}}))
              (local tool (misa.tool block.name))
              (if tool
                  (do
                    (assert (not (. agent.pending_tools block.id))
                            "duplicate tool call id")
                    (set agent (misa.patch agent {:pending_tools {block.id {:name block.name :request_id id}}
                                                  :pending_tool_count (+ agent.pending_tool_count 1)}))
                    (if (. argument-failures block.id)
                        (tset effects (+ (length effects) 1)
                              {:event {:is_error true
                                       :text (. argument-failures block.id)
                                       :tool_call_id block.id
                                       :type :tool/result}
                               :type :dispatch})
                        (tset effects (+ (length effects) 1)
                              {:arguments block.arguments
                               :name block.name
                               :request_id id
                               :tool_call_id block.id
                               :type tool.effect})))
                  (let [message (.. "unknown tool: " block.name)]
                    (set agent (misa.patch agent {:tool_batch {:results {block.id (tool-result block.id message true)}}}))
                    (tset effects (+ (length effects) 1)
                          {:event {:id block.id
                                   :is_error true
                                   :text message
                                   :type :transcript/tool-result}
                           :type :dispatch})))))))
    (if (> agent.pending_tool_count 0)
        (do
          (set agent (misa.patch agent {:status :tools}))
          (tset effects (+ (length effects) 1)
                {:event {:status :tools :type :agent/status} :type :dispatch}))
        saw-tool
        (do
          (set agent (flush-tool-results agent))
          (local (next provider problem) (request db agent))
          (set agent next)
          (if problem
              (let [(ready blocked-fx) (blocked agent problem)]
                (set agent ready)
                (each [_ effect (ipairs blocked-fx)] (table.insert effects effect)))
              (do
                (tset effects (+ (length effects) 1)
                      {:event {:status :working :type :agent/status}
                       :type :dispatch})
                (tset effects (+ (length effects) 1) provider))))
        (do
          (set agent (misa.patch agent {:status :ready}))
          (tset effects (+ (length effects) 1)
                {:event {:status :ready :type :agent/status} :type :dispatch})
          (tset effects (+ (length effects) 1)
                {:event {:exit agent.exit_after_response
                         : id
                         :type :agent/completed}
                 :type :dispatch})))
    {:patch {:agent (misa.replace agent)} :fx effects}))

(fn empty-usage []
  {:cache_read_tokens 0 :cache_write_tokens 0 :input_tokens 0 :output_tokens 0})

(fn pending-ids [agent]
  (local ids (icollect [id (pairs agent.pending_tools)] id))
  (table.sort ids)
  ids)

(fn cancelled [_ agent id interrupt-response]
  (let [response-id (or (or id agent.active_request_id)
                        agent.accepted_request_id)]
    (local messages (icollect [_ message (ipairs agent.messages)] message))
    (when agent.stream
      (local partial-content (icollect [_ block (ipairs agent.stream.blocks)]
                               (when (and (= block.type :text) (> (length block.chunks) 0))
                                 {:text (table.concat block.chunks) :type :text})))
      (when (> (length partial-content) 0)
        (table.insert messages {:content partial-content :role :assistant})))
    (when agent.tool_batch
      (each [_ call-id (ipairs agent.tool_batch.order)]
        (table.insert messages (or (. agent.tool_batch.results call-id)
                                   (tool-result call-id :Cancelled true)))))
    (local effects {})
    (when (and interrupt-response response-id)
      (tset effects (+ (length effects) 1)
            {:event {:response_id response-id
                     :type :transcript/response-interrupted}
             :type :dispatch}))
    (tset effects (+ (length effects) 1)
          {:event {:level :warning :text :Cancelled :type :transcript/harness}
           :type :dispatch})
    (tset effects (+ (length effects) 1)
          {:event {:status :ready :type :agent/status} :type :dispatch})
    (tset effects (+ (length effects) 1)
          {:event {:exit agent.exit_after_response
                   :id response-id
                   :type :agent/completed}
           :type :dispatch})
    {:patch {:agent {:messages (misa.replace messages) :error misa.delete :status :ready
                      :active_request_id misa.delete :stream misa.delete :cancel_requested false
                      :pending_tools (misa.replace {}) :pending_tool_count 0 :tool_batch misa.delete}}
     :fx effects}))

(fn stream-blocks [stream]
  (let [blocks {}]
    (each [_ source (ipairs stream.blocks)]
      (when (and (= source.type :tool_call)
                 (not (and (and (and (= (type source.id) :string)
                                     (not= source.id ""))
                                (= (type source.name) :string))
                           (not= source.name ""))))
        (lua "return nil"))
      (local block {:execution source.execution
                    :transcript_id source.transcript_id
                    :type source.type})
      (if (or (= source.type :text) (= source.type :thinking))
          (set block.text (table.concat source.chunks))
          (do
            (set (block.id block.name block.arguments)
                 (values source.id source.name source.arguments))
            (set block.arguments_json
                 (or (and source.arguments_json_chunks
                          (table.concat source.arguments_json_chunks))
                     source.arguments_json))))
      (tset blocks (+ (length blocks) 1) block))
    blocks))

(fn transcript-event [id kind fields]
  {:type :dispatch :event (misa.patch (or fields {}) {:type kind :response_id id})})

(fn stream-block [stream block kind id]
  (local sequence (+ stream.block_seq 1))
  (local next-block (misa.patch block {:transcript_id (.. id "/" sequence)}))
  (values (misa.patch stream {:block_seq sequence :blocks (misa.replace (appended stream.blocks next-block))})
          next-block
          (transcript-event id :transcript/block-start
                            {:block_id next-block.transcript_id :call_id next-block.id
                             :name next-block.name : kind})))

(fn replaced-block [stream index block]
  (icollect [i previous (ipairs stream.blocks)] (if (= i index) block previous)))

(fn text-delta [previous delta id]
  (assert (= (type delta.text) :string) "stream text delta must be a string")
  (when (not= delta.text "")
    (var stream previous)
    (var block (. stream.blocks (length stream.blocks)))
    (local fx [])
    (when (or (not block) (not= block.type delta.type))
      (local (next created effect) (stream-block stream {:type delta.type :chunks []}
                                                 (if (= delta.type :text) :assistant :thinking) id))
      (set stream next)
      (set block created)
      (table.insert fx effect))
    (local next-block (misa.patch block {:chunks (misa.replace (appended block.chunks delta.text))}))
    (table.insert fx (transcript-event id :transcript/block-delta
                                      {:block_id block.transcript_id :text delta.text}))
    {:patch {:block_seq stream.block_seq
             :blocks (misa.replace (replaced-block stream (length stream.blocks) next-block))}
     : fx}))

(fn tool-delta [previous delta id]
  (local key (tostring (or delta.index delta.id (+ (length previous.blocks) 1))))
  (var stream previous)
  (var index (. stream.tools key))
  (var block (and index (. stream.blocks index)))
  (local fx [])
  (when (not block)
    (local (next created effect) (stream-block stream {:type :tool_call :id delta.id :name delta.name
                                                     :arguments_json_chunks []} :tool_call id))
    (set stream next)
    (set block created)
    (set index (length stream.blocks))
    (table.insert fx effect))
  (var chunks (if (not= delta.arguments nil) nil block.arguments_json_chunks))
  (when (not= delta.arguments_json nil) (set chunks [delta.arguments_json]))
  (when (not= delta.arguments_json_delta nil)
    (set chunks (appended chunks delta.arguments_json_delta)))
  (local next-block (misa.patch block {:execution delta.execution :id delta.id :name delta.name
                                       :arguments (when (not= delta.arguments nil) (misa.replace delta.arguments))
                                       :arguments_json_chunks (misa.replace chunks)}))
  (table.insert fx (transcript-event id :transcript/block-delta
                                    {:block_id block.transcript_id :call_id delta.id :name delta.name
                                     :arguments delta.arguments :arguments_json delta.arguments_json
                                     :arguments_json_delta delta.arguments_json_delta}))
  {:patch {:block_seq stream.block_seq :tools {key index}
           :blocks (misa.replace (replaced-block stream index next-block))}
   : fx})

(fn active-stream [db event]
  (local agent db.agent)
  (local stream (and agent agent.stream))
  (when (and stream (not agent.cancel_requested)
             (= event.id agent.active_request_id) (= event.id stream.id))
    stream))

{:setup (fn [context]
          (local setup-fx [])
          (local deltas {:text text-delta :thinking text-delta :tool_call tool-delta})
          (table.insert setup-fx {:type :register/setup-effect :name :register/agent-delta
                                 :handler (fn [effect]
                                            (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                         (= (type effect.value) :function) (not (. deltas effect.id)))
                                                    "invalid or duplicate agent delta handler")
                                            (tset deltas effect.id effect.value))})
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "Reset conversation and token usage"
                                 :event :agent/reset
                                 :name :/clear}})
          (var config (or (and (= (type context.config) :table)
                               context.config.agent)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (assert (or (= config.system_prompt nil)
                      (= (type config.system_prompt) :string))
                  "config.agent.system_prompt must be a string")
          (table.insert setup-fx
                        {:type :register/event :name :app/start
                         :handler (fn [db _ cofx]
                                    (local has-prompt (> (length cofx.argv) 0))
                                    (local waiting (and db.auth_startup (not db.auth_startup.ready)))
                                    (local prompt (when has-prompt (table.concat cofx.argv " ")))
                                    {:patch {:agent (misa.replace
                                                      {:exit_after_response has-prompt :messages []
                                                       :pending_tool_count 0 :pending_tools {} :request_seq 0
                                                       :status :ready :system_prompt config.system_prompt
                                                       :startup_prompt (when waiting prompt)
                                                       :usage (empty-usage)})}
                                     :fx (if (and has-prompt (not waiting))
                                             [{:type :dispatch :event {:type :agent/submit : prompt}}] [])})})
          (table.insert setup-fx
                        {:type :register/event :name :auth/startup-ready
                         :handler (fn [db]
                                    (local agent db.agent)
                                    (when (and agent agent.startup_prompt)
                                      {:patch {:agent {:startup_prompt misa.delete :startup_attachments misa.delete}}
                                       :fx [{:type :dispatch :event {:type :agent/submit
                                                                    :prompt agent.startup_prompt
                                                                    :attachments agent.startup_attachments}}]}))})
          (table.insert setup-fx
                        {:type :register/event :name :agent/cancel-active
                         :handler (fn [db]
                                    (local agent db.agent)
                                    (when (and agent (not agent.cancel_requested))
                                      (local ids (if (and (= agent.status :working) agent.active_request_id)
                                                     [agent.active_request_id]
                                                     (and (= agent.status :tools) (> agent.pending_tool_count 0))
                                                     (pending-ids agent) []))
                                      (when (> (length ids) 0)
                                        (local fx [{:type :dispatch :event {:type :agent/status :status :cancelling}}])
                                        (each [_ id (ipairs ids)] (table.insert fx {:type :operation/cancel : id}))
                                        {:patch {:agent {:cancel_requested true :status :cancelling}} : fx})))})
          (table.insert setup-fx
                        {:type :register/event :name :agent/reset
                         :handler (fn [db]
                                    (local agent (assert db.agent "agent state is not initialized"))
                                    (local fx [])
                                    (when agent.active_request_id
                                      (table.insert fx {:type :operation/cancel :id agent.active_request_id}))
                                    (each [_ id (ipairs (pending-ids agent))]
                                      (table.insert fx {:type :operation/cancel : id}))
                                    (table.insert fx {:type :dispatch :event {:type :transcript/reset}})
                                    (table.insert fx {:type :dispatch :event {:type :agent/status :status :ready
                                                                            :last_usage {} :usage (empty-usage)}})
                                    {:patch {:agent {:status :ready :active_request_id misa.delete
                                                     :accepted_request_id misa.delete :stream misa.delete
                                                     :cancel_requested false :pending_tools (misa.replace {})
                                                     :pending_tool_count 0 :tool_batch misa.delete
                                                     :messages (misa.replace []) :error misa.delete
                                                     :startup_prompt misa.delete :startup_attachments misa.delete
                                                     :last_usage (misa.replace {}) :usage (misa.replace (empty-usage))}}
                                     : fx})})
          (table.insert setup-fx
                        {:type :register/event :name :agent/submit
                         :handler (fn [db event]
                                    (assert (and (= (type event.prompt) :string)
                                                 (or (not= event.prompt "") (> (length (or event.attachments [])) 0)))
                                            "agent prompt must be nonempty")
                                    (local agent (assert db.agent "agent state is not initialized"))
                                    (if (not= agent.status :ready) nil
                                        (and db.auth_startup (not db.auth_startup.ready))
                                        {:patch {:agent {:startup_prompt event.prompt
                                                         :startup_attachments (misa.replace event.attachments)}}}
                                        (not (selected-model db))
                                        (let [configured (and db.models db.models.configured_default)
                                              message (if configured (.. "configured model is unavailable: " configured)
                                                          "no available models; log in to a provider")
                                              problem {:code :missing_model :kind :request_readiness : message :model configured}]
                                          {:fx [{:type :dispatch :event {:type :transcript/harness :level :error : problem :text message}}
                                                {:type :dispatch :event {:type :agent/completed :exit agent.exit_after_response}}]})
                                        (do
                                          (local blocks (if (= event.prompt "") [] [{:type :text :text event.prompt}]))
                                          (each [_ image (ipairs (or event.attachments []))]
                                            (assert (and (= image.type :image) (= (type image.source) :table)
                                                         (= image.source.type :base64) (= (type image.source.media_type) :string)
                                                         (= (type image.source.data) :string)) "invalid image attachment")
                                            (table.insert blocks image))
                                          (local history (misa.patch agent {:messages (misa.replace (appended agent.messages
                                                                                                             {:content blocks :role :user}))}))
                                          (local (next provider problem) (request db history))
                                          (if problem
                                              (let [(ready fx) (blocked agent problem)]
                                                {:patch {:agent (misa.replace ready)} : fx})
                                              {:patch {:agent (misa.replace next)}
                                               :fx [{:type :dispatch :event {:type :agent/submitted :prompt event.prompt}}
                                                    {:type :dispatch :event {:type :transcript/user :text event.prompt :attachments event.attachments}}
                                                    {:type :dispatch :event {:type :agent/status :status :working}}
                                                    provider]}))))})
          ;; Correlation is shared policy; delta assembly is an open pure dispatcher.
          (table.insert setup-fx
                        {:type :register/event :name :agent/stream-start
                         :handler (fn [db event]
                                    (local agent db.agent)
                                    (when (and agent (= agent.status :working)
                                               (= event.id agent.active_request_id) (not agent.stream))
                                      {:patch {:agent {:stream (misa.replace {:id event.id :block_seq 0
                                                                             :blocks [] :tools {} :tool_results {}})}}
                                       :fx [(transcript-event event.id :transcript/response-start
                                                              {:model agent.request_model :role :assistant})]}))})
          (table.insert setup-fx
                        {:type :register/event :name :agent/stream-delta
                         :handler (fn [db event]
                                    (local stream (active-stream db event))
                                    (when (and stream (= (type event.delta) :table))
                                      (local handler (assert (. deltas event.delta.type)
                                                             (.. "unsupported agent stream delta: " (tostring event.delta.type))))
                                      (local result (handler stream event.delta event.id))
                                      (when result {:patch {:agent {:stream (or result.patch {})}} :fx result.fx})))})
          ;; Provider-owned tools are observations, never local executions.
          (table.insert setup-fx
                        {:type :register/event :name :agent/stream-tool-result
                         :handler (fn [db event]
                                    (when (active-stream db event)
                                      (assert (= (type event.tool_call_id) :string)
                                              "provider tool result requires a call id")
                                      {:patch {:agent {:stream {:tool_results
                                                               {event.tool_call_id
                                                                (misa.replace (tool-result event.tool_call_id
                                                                                           (or event.text "") event.is_error))}}}}
                                       :fx [{:type :dispatch :event {:type :transcript/tool-result
                                                                    :id event.tool_call_id
                                                                    :is_error (= event.is_error true)
                                                                    :text (or event.text "")}}]}))})
          (table.insert setup-fx
                        {:type :register/event :name :agent/stream-usage
                         :handler (fn [db event]
                                    (when (active-stream db event)
                                      (local value (if (= (type event.usage) :table) event.usage {}))
                                      {:patch {:agent {:stream {:stop_reason event.stop_reason
                                                               :usage {:input_tokens value.input_tokens
                                                                       :output_tokens value.output_tokens
                                                                       :cache_read_tokens value.cache_read_tokens
                                                                       :cache_write_tokens value.cache_write_tokens
                                                                       :cost_usd value.cost_usd
                                                                       :input_includes_cache value.input_includes_cache}}}}}))})
          ;; Opaque continuation state stays separate from visible text.
          (table.insert setup-fx
                        {:type :register/event :name :agent/stream-state
                         :handler (fn [db event]
                                    (local stream (active-stream db event))
                                    (when stream
                                      (assert (and (= (type event.provider) :string) (= (type event.value) :table))
                                              "invalid provider continuation state")
                                      {:patch {:agent {:stream {:provider_state
                                                               (misa.replace (appended stream.provider_state
                                                                                       {:provider event.provider :value event.value}))}}}}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/stream-end
                         :handler (fn [db event]
                                    (local agent db.agent)
                                    (local stream (and agent agent.stream))
                                    (if (or (or (not stream)
                                                (not= event.id
                                                      agent.active_request_id))
                                            (not= stream.id event.id))
                                        nil
                                        (if agent.cancel_requested
                                            (cancelled db agent event.id true)
                                            (if (not= agent.status :working)
                                                nil
                                                (do
                                                  (local blocks
                                                         (stream-blocks stream))
                                                  (if (not blocks)
                                                      {:fx [{:event {:id event.id
                                                                     :message "provider ended an incomplete tool call"
                                                                     :type :agent/stream-error}
                                                             :type :dispatch}]}
                                                      (complete-response db
                                                                         agent
                                                                         event.id
                                                                         blocks
                                                                         (or event.usage
                                                                             stream.usage)
                                                                         (or event.stop_reason
                                                                             stream.stop_reason))))))))})
          ;; Compatibility input is immediately normalized; built-in providers never
          ;; use this legacy event.
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/result
                         :handler (fn [db event]
                                    (local agent db.agent)
                                    (if (or (not agent)
                                            (not= event.id
                                                  agent.active_request_id))
                                        nil
                                        (if agent.cancel_requested
                                            (cancelled db agent event.id
                                                       (not= agent.stream nil))
                                            (if (not= agent.status :working)
                                                nil
                                                (complete-response db agent
                                                                   event.id
                                                                   event.content
                                                                   event.usage
                                                                   event.stop_reason)))))})
          (table.insert setup-fx
                        {:type :register/event :name :tool/result
                         :handler (fn [db event]
                                    (local previous db.agent)
                                    (when (and previous (. previous.pending_tools event.tool_call_id))
                                      (var agent (misa.patch previous
                                                            {:pending_tools {event.tool_call_id misa.delete}
                                                             :pending_tool_count (- previous.pending_tool_count 1)}))
                                      (local update {:type :dispatch
                                                     :event {:type :transcript/tool-result :id event.tool_call_id
                                                             :cancelled (when agent.cancel_requested true)
                                                             :is_error (= event.is_error true)
                                                             :text (or event.text (if agent.cancel_requested :Cancelled ""))}})
                                      (if agent.cancel_requested
                                          (if (= agent.pending_tool_count 0)
                                              (let [result (cancelled db agent agent.accepted_request_id true)]
                                                (table.insert result.fx 1 update)
                                                result)
                                              {:patch {:agent (misa.replace agent)} :fx [update]})
                                          (do
                                            (assert (and agent.tool_batch (not (. agent.tool_batch.results event.tool_call_id)))
                                                    "duplicate tool result")
                                            (set agent (misa.patch agent
                                                                  {:tool_batch {:results
                                                                                {event.tool_call_id
                                                                                 (tool-result event.tool_call_id (or event.text "") event.is_error)}}}))
                                            (local fx [update])
                                            (when (= agent.pending_tool_count 0)
                                              (set agent (flush-tool-results agent))
                                              (local (next provider problem) (request db agent))
                                              (set agent next)
                                              (if problem
                                                  (let [(ready blocked-fx) (blocked agent problem)]
                                                    (set agent ready)
                                                    (each [_ effect (ipairs blocked-fx)] (table.insert fx effect)))
                                                  (do
                                                    (table.insert fx {:type :dispatch :event {:type :agent/status :status :working}})
                                                    (table.insert fx provider))))
                                            {:patch {:agent (misa.replace agent)} : fx}))))})

          (fn stream-error [db event]
            (local agent db.agent)
            (if (or (or (not agent) (not= event.id agent.active_request_id))
                    (and (not= agent.status :working)
                         (not= agent.status :cancelling)))
                nil
                (if agent.cancel_requested
                    (cancelled db agent event.id (not= agent.stream nil))
                    (do
                      (local had-stream (not= agent.stream nil))
                      (local message (tostring (or event.message "provider failed")))
                      (local fx {})
                      (when had-stream
                        (tset fx (+ (length fx) 1)
                              {:event {:response_id event.id
                                       :type :transcript/response-interrupted}
                               :type :dispatch}))
                      (tset fx (+ (length fx) 1)
                            {:event {:level :error
                                     :text message
                                     :type :transcript/harness}
                             :type :dispatch})
                      (tset fx (+ (length fx) 1)
                            {:event {:status :ready :type :agent/status}
                             :type :dispatch})
                      (tset fx (+ (length fx) 1)
                            {:event {:exit agent.exit_after_response
                                     :id event.id
                                     :type :agent/completed}
                             :type :dispatch})
                      {:patch {:agent {:error message :status :ready :active_request_id misa.delete
                                       :stream misa.delete :accepted_request_id event.id}}
                       : fx}))))

          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/stream-error
                         :handler stream-error})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/error
                         :handler stream-error})
          {:fx setup-fx})}
