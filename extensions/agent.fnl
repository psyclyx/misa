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

(fn request [db]
  (let [agent db.agent
        selected (assert (selected-model db)
                         "selected model became unavailable")]
    (var (options problem) (values {} nil))
    (when misa.prepare_request_options
      (set (options problem) (misa.prepare_request_options db selected)))
    (if problem (values nil problem)
        (do
          (set agent.request_seq (+ agent.request_seq 1))
          (local id (.. :agent- (tostring agent.request_seq)))
          (set (agent.active_request_id agent.status agent.cancel_requested)
               (values id :working false))
          (set agent.request_model selected.id)
          {: id
           :messages agent.messages
           :model selected.model
           :request_options options
           :system_prompt agent.system_prompt
           :tools (misa.tools)
           :type (.. :provider. selected.provider)}))))

(fn blocked [agent problem]
  (set (agent.status agent.active_request_id) (values :ready nil))
  [{:event {:level :error
            : problem
            :text problem.message
            :type :transcript/harness}
    :type :dispatch}
   {:event {:status :ready :type :agent/status} :type :dispatch}
   {:event {:exit agent.exit_after_response :type :agent/completed}
    :type :dispatch}])

(fn record-usage [agent usage]
  (if (= usage nil) nil (do
                          (assert (= (type usage) :table)
                                  "usage must be a table")
                          (local normalized {})
                          (each [_ name (ipairs [:input_tokens
                                                 :output_tokens
                                                 :cache_read_tokens
                                                 :cache_write_tokens])]
                            (local value (or (. usage name) 0))
                            (assert (and (and (= (type value) :number)
                                              (>= value 0))
                                         (= (% value 1) 0))
                                    (.. name " must be a nonnegative integer"))
                            (tset normalized name value)
                            (tset agent.usage name
                                  (+ (. agent.usage name) value)))
                          (set agent.last_usage normalized)
                          (set normalized.input_includes_cache
                               usage.input_includes_cache)
                          (when (and (= (type usage.cost_usd) :number)
                                     (>= usage.cost_usd 0))
                            (set normalized.cost_usd usage.cost_usd))
                          nil)))

(fn tool-result [call-id text is-error]
  {:content [{:text (tostring text) :type :text}]
   :is_error (= is-error true)
   :role :tool
   :tool_call_id call-id})

(fn flush-tool-results [agent]
  (let [batch (assert agent.tool_batch "tool result batch is missing")]
    (each [_ call-id (ipairs batch.order)]
      (tset agent.messages (+ (length agent.messages) 1)
            (assert (. batch.results call-id)
                    (.. "tool result is missing: " call-id))))
    (set agent.tool_batch nil)
    nil))

(fn normalize-tool-arguments [blocks]
  (let [failures {}]
    (each [_ block (ipairs blocks)]
      (when (= block.type :tool_call)
        (when (= block.arguments nil)
          (assert (= (type block.arguments_json) :string)
                  "tool call arguments are missing")
          (assert (and misa.json (= (type misa.json.decode) :function))
                  "JSON extension is required for provider tool calls")
          (local (ok decoded)
                 (pcall misa.json.decode
                        (or (and (= block.arguments_json "") "{}")
                            block.arguments_json)))
          (if (and ok (= (type decoded) :table)) (set block.arguments decoded)
              (do
                (set block.arguments {})
                (tset failures block.id
                      (or (and ok "tool arguments must be a JSON object")
                          (tostring decoded))))))
        (set block.arguments_json nil)))
    failures))

(fn complete-response [db agent id blocks usage stop-reason]
  (let [stream agent.stream
        argument-failures (normalize-tool-arguments blocks)]
    (set-forcibly! blocks (content blocks :assistant))
    (record-usage agent usage)
    (tset agent.messages (+ (length agent.messages) 1)
          {:content blocks
           :provider_state (and stream stream.provider_state)
           :role :assistant})
    (set (agent.active_request_id agent.accepted_request_id agent.stream)
         (values nil id nil))
    (var (effects saw-tool) (values {} false))
    (set agent.tool_batch nil)
    (if stream (each [_ block (ipairs blocks)]
                 (tset effects (+ (length effects) 1)
                       {:event {:arguments block.arguments
                                :block_id block.transcript_id
                                :call_id block.id
                                :name block.name
                                :response_id id
                                :type :transcript/block-end}
                        :type :dispatch})
                 (set block.transcript_id nil))
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
              (set block.execution nil)
              (var result
                   (and (and stream stream.tool_results)
                        (. stream.tool_results block.id)))
              (set result (or result
                              (tool-result block.id
                                           "Provider did not report a tool result"
                                           true)))
              (tset agent.messages (+ (length agent.messages) 1) result)
              (tset effects (+ (length effects) 1)
                    {:event {:id block.id
                             :is_error result.is_error
                             :text (. result.content 1 :text)
                             :type :transcript/tool-result}
                     :type :dispatch}))
            (do
              (set saw-tool true)
              (when (not agent.tool_batch)
                (set agent.tool_batch {:order {} :results {}}))
              (assert (not (. agent.tool_batch.results block.id))
                      "duplicate tool call id")
              (tset agent.tool_batch.order
                    (+ (length agent.tool_batch.order) 1) block.id)
              (local tool (misa.tool block.name))
              (if tool
                  (do
                    (assert (not (. agent.pending_tools block.id))
                            "duplicate tool call id")
                    (tset agent.pending_tools block.id
                          {:name block.name :request_id id})
                    (set agent.pending_tool_count
                         (+ agent.pending_tool_count 1))
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
                    (tset agent.tool_batch.results block.id
                          (tool-result block.id message true))
                    (tset effects (+ (length effects) 1)
                          {:event {:id block.id
                                   :is_error true
                                   :text message
                                   :type :transcript/tool-result}
                           :type :dispatch})))))))
    (if (> agent.pending_tool_count 0)
        (do
          (set agent.status :tools)
          (tset effects (+ (length effects) 1)
                {:event {:status :tools :type :agent/status} :type :dispatch}))
        saw-tool
        (do
          (flush-tool-results agent)
          (local (provider problem) (request db))
          (if problem
              (each [_ effect (ipairs (blocked agent problem))]
                (tset effects (+ (length effects) 1) effect))
              (do
                (tset effects (+ (length effects) 1)
                      {:event {:status :working :type :agent/status}
                       :type :dispatch})
                (tset effects (+ (length effects) 1) provider))))
        (do
          (set agent.status :ready)
          (tset effects (+ (length effects) 1)
                {:event {:status :ready :type :agent/status} :type :dispatch})
          (tset effects (+ (length effects) 1)
                {:event {:exit agent.exit_after_response
                         : id
                         :type :agent/completed}
                 :type :dispatch})))
    {: db :fx effects}))

(fn cancelled [db agent id interrupt-response]
  (let [response-id (or (or id agent.active_request_id)
                        agent.accepted_request_id)]
    (when agent.stream
      (local ___partial___ {})
      (each [_ block (ipairs agent.stream.blocks)]
        (when (and (= block.type :text) (> (length block.chunks) 0))
          (tset ___partial___ (+ (length ___partial___) 1)
                {:text (table.concat block.chunks) :type :text})))
      (when (> (length ___partial___) 0)
        (tset agent.messages (+ (length agent.messages) 1)
              {:content ___partial___ :role :assistant})))
    (when agent.tool_batch
      (each [_ call-id (ipairs agent.tool_batch.order)]
        (tset agent.tool_batch.results call-id
              (or (. agent.tool_batch.results call-id)
                  (tool-result call-id :Cancelled true))))
      (flush-tool-results agent))
    (set (agent.error agent.status agent.active_request_id agent.stream
                      agent.cancel_requested)
         (values nil :ready nil nil false))
    (set (agent.pending_tools agent.pending_tool_count agent.tool_batch)
         (values {} 0 nil))
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
    {: db :fx effects}))

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

(fn appended [items value]
  (local next (icollect [_ item (ipairs (or items []))] item))
  (table.insert next value)
  next)

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
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db _ cofx]
                                    (set db.agent
                                         {:exit_after_response (> (length cofx.argv)
                                                                  0)
                                          :messages {}
                                          :pending_tool_count 0
                                          :pending_tools {}
                                          :request_seq 0
                                          :status :ready
                                          :system_prompt config.system_prompt
                                          :usage {:cache_read_tokens 0
                                                  :cache_write_tokens 0
                                                  :input_tokens 0
                                                  :output_tokens 0}})
                                    (if (= (length cofx.argv) 0) {: db}
                                        (do
                                          (local prompt
                                                 (table.concat cofx.argv " "))
                                          (if (and db.auth_startup
                                                   (not db.auth_startup.ready))
                                              (do
                                                (set db.agent.startup_prompt
                                                     prompt)
                                                {: db})
                                              {: db
                                               :fx [{:event {: prompt
                                                             :type :agent/submit}
                                                     :type :dispatch}]}))))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :auth/startup-ready
                         :handler (fn [db]
                                    (local agent db.agent)
                                    (if (or (not agent)
                                            (not agent.startup_prompt))
                                        nil
                                        (do
                                          (local prompt agent.startup_prompt)
                                          (local attachments
                                                 agent.startup_attachments)
                                          (set (agent.startup_prompt agent.startup_attachments)
                                               (values nil nil))
                                          {: db
                                           :fx [{:event {: attachments
                                                         : prompt
                                                         :type :agent/submit}
                                                 :type :dispatch}]})))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/cancel-active
                         :handler (fn [db]
                                    (local agent db.agent)
                                    (if (or (not agent) agent.cancel_requested)
                                        {: db}
                                        (do
                                          (var effects {})
                                          (if (and (= agent.status :working)
                                                   agent.active_request_id)
                                              (do
                                                (set (agent.cancel_requested agent.status)
                                                     (values true :cancelling))
                                                (set effects
                                                     [{:event {:status :cancelling
                                                               :type :agent/status}
                                                       :type :dispatch}
                                                      {:id agent.active_request_id
                                                       :type :operation/cancel}]))
                                              (and (= agent.status :tools)
                                                   (> agent.pending_tool_count
                                                      0))
                                              (do
                                                (set (agent.cancel_requested agent.status)
                                                     (values true :cancelling))
                                                (tset effects 1
                                                      {:event {:status :cancelling
                                                               :type :agent/status}
                                                       :type :dispatch})
                                                (local ids {})
                                                (each [id (pairs agent.pending_tools)]
                                                  (tset ids (+ (length ids) 1)
                                                        id))
                                                (table.sort ids)
                                                (each [_ id (ipairs ids)]
                                                  (tset effects
                                                        (+ (length effects) 1)
                                                        {: id
                                                         :type :operation/cancel}))))
                                          {: db :fx effects})))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/reset
                         :handler (fn [db]
                                    (local agent
                                           (assert db.agent
                                                   "agent state is not initialized"))
                                    (local effects {})
                                    (when agent.active_request_id
                                      (tset effects (+ (length effects) 1)
                                            {:id agent.active_request_id
                                             :type :operation/cancel}))
                                    (each [id (pairs agent.pending_tools)]
                                      (tset effects (+ (length effects) 1)
                                            {: id :type :operation/cancel}))
                                    (set (agent.status agent.active_request_id
                                                       agent.accepted_request_id
                                                       agent.stream
                                                       agent.cancel_requested)
                                         (values :ready nil nil nil false))
                                    (set (agent.pending_tools agent.pending_tool_count
                                                              agent.tool_batch)
                                         (values {} 0 nil))
                                    (set (agent.messages agent.error
                                                         agent.last_usage)
                                         (values {} nil {}))
                                    (set agent.usage
                                         {:cache_read_tokens 0
                                          :cache_write_tokens 0
                                          :input_tokens 0
                                          :output_tokens 0})
                                    (tset effects (+ (length effects) 1)
                                          {:event {:type :transcript/reset}
                                           :type :dispatch})
                                    (tset effects (+ (length effects) 1)
                                          {:event {:last_usage agent.last_usage
                                                   :status :ready
                                                   :type :agent/status
                                                   :usage agent.usage}
                                           :type :dispatch})
                                    {: db :fx effects})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/submit
                         :handler (fn [db event]
                                    (assert (and (= (type event.prompt) :string)
                                                 (or (not= event.prompt "")
                                                     (> (length (or event.attachments
                                                                    {}))
                                                        0)))
                                            "agent prompt must be nonempty")
                                    (local agent
                                           (assert db.agent
                                                   "agent state is not initialized"))
                                    (if (not= agent.status :ready) {: db}
                                        (if (and db.auth_startup
                                                 (not db.auth_startup.ready))
                                            (do
                                              (set (agent.startup_prompt agent.startup_attachments)
                                                   (values event.prompt
                                                           event.attachments))
                                              {: db})
                                            (if (not (selected-model db))
                                                (do
                                                  (local configured
                                                         (and db.models
                                                              db.models.configured_default))
                                                  (local message
                                                         (or (and configured
                                                                  (.. "configured model is unavailable: "
                                                                      configured))
                                                             "no available models; log in to a provider"))
                                                  (local problem
                                                         {:code :missing_model
                                                          :kind :request_readiness
                                                          : message
                                                          :model configured})
                                                  {: db
                                                   :fx [{:event {:level :error
                                                                 : problem
                                                                 :text message
                                                                 :type :transcript/harness}
                                                         :type :dispatch}
                                                        {:event {:exit agent.exit_after_response
                                                                 :type :agent/completed}
                                                         :type :dispatch}]})
                                                (do
                                                  (local (provider problem)
                                                         (request db))
                                                  (if problem
                                                      {: db
                                                       :fx (blocked agent
                                                                    problem)}
                                                      (do
                                                        (local blocks {})
                                                        (when (not= event.prompt
                                                                    "")
                                                          (tset blocks
                                                                (+ (length blocks)
                                                                   1)
                                                                {:text event.prompt
                                                                 :type :text}))
                                                        (each [_ image (ipairs (or event.attachments
                                                                                   {}))]
                                                          (assert (and (and (and (and (= image.type
                                                                                         :image)
                                                                                      (= (type image.source)
                                                                                         :table))
                                                                                 (= image.source.type
                                                                                    :base64))
                                                                            (= (type image.source.media_type)
                                                                               :string))
                                                                       (= (type image.source.data)
                                                                          :string))
                                                                  "invalid image attachment")
                                                          (tset blocks
                                                                (+ (length blocks)
                                                                   1)
                                                                image))
                                                        (tset agent.messages
                                                              (+ (length agent.messages)
                                                                 1)
                                                              {:content blocks
                                                               :role :user})
                                                        {: db
                                                         :fx [{:event {:prompt event.prompt
                                                                       :type :agent/submitted}
                                                               :type :dispatch}
                                                              {:event {:attachments event.attachments
                                                                       :text event.prompt
                                                                       :type :transcript/user}
                                                               :type :dispatch}
                                                              {:event {:status :working
                                                                       :type :agent/status}
                                                               :type :dispatch}
                                                              provider]})))))))})
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
                                                      {: db
                                                       :fx [{:event {:id event.id
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
                        {:type :register/event
                         :name :tool/result
                         :handler (fn [db event]
                                    (local agent db.agent)
                                    ;; Cancellation/reset can race an already queued completion.
                                    (if (or (not agent)
                                            (not (. agent.pending_tools
                                                    event.tool_call_id)))
                                        nil
                                        (do
                                          (tset agent.pending_tools
                                                event.tool_call_id nil)
                                          (set agent.pending_tool_count
                                               (- agent.pending_tool_count 1))
                                          (if agent.cancel_requested
                                              (do
                                                (local update
                                                       {:event {:cancelled true
                                                                :id event.tool_call_id
                                                                :is_error (= event.is_error
                                                                             true)
                                                                :text (or event.text
                                                                          :Cancelled)
                                                                :type :transcript/tool-result}
                                                        :type :dispatch})
                                                (if (= agent.pending_tool_count
                                                       0)
                                                    (do
                                                      (local tx
                                                             (cancelled db
                                                                        agent
                                                                        agent.accepted_request_id
                                                                        true))
                                                      (table.insert tx.fx 1
                                                                    update)
                                                      tx)
                                                    {: db :fx [update]}))
                                              (do
                                                (local result
                                                       (tool-result event.tool_call_id
                                                                    (or event.text
                                                                        "")
                                                                    event.is_error))
                                                (assert (and agent.tool_batch
                                                             (not (. agent.tool_batch.results
                                                                     event.tool_call_id)))
                                                        "duplicate tool result")
                                                (tset agent.tool_batch.results
                                                      event.tool_call_id result)
                                                (local effects
                                                       [{:event {:id event.tool_call_id
                                                                 :is_error (= event.is_error
                                                                              true)
                                                                 :text (or event.text
                                                                           "")
                                                                 :type :transcript/tool-result}
                                                         :type :dispatch}])
                                                (when (= agent.pending_tool_count
                                                         0)
                                                  (flush-tool-results agent)
                                                  (local (provider problem)
                                                         (request db))
                                                  (if problem
                                                      (each [_ effect (ipairs (blocked agent
                                                                                       problem))]
                                                        (tset effects
                                                              (+ (length effects)
                                                                 1)
                                                              effect))
                                                      (do
                                                        (tset effects
                                                              (+ (length effects)
                                                                 1)
                                                              {:event {:status :working
                                                                       :type :agent/status}
                                                               :type :dispatch})
                                                        (tset effects
                                                              (+ (length effects)
                                                                 1)
                                                              provider))))
                                                {: db :fx effects})))))})

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
                      (set (agent.error agent.status agent.active_request_id
                                        agent.stream)
                           (values (tostring (or event.message
                                                 "provider failed"))
                                   :ready nil nil))
                      (set agent.accepted_request_id event.id)
                      (local fx {})
                      (when had-stream
                        (tset fx (+ (length fx) 1)
                              {:event {:response_id event.id
                                       :type :transcript/response-interrupted}
                               :type :dispatch}))
                      (tset fx (+ (length fx) 1)
                            {:event {:level :error
                                     :text agent.error
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
                      {: db : fx}))))

          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/stream-error
                         :handler stream-error})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/error
                         :handler stream-error})
          {:fx setup-fx})}
