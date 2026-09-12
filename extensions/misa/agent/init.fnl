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
    (accumulate [selected nil _ model (ipairs (or state.entries []))]
      (or selected (when (= model.id state.selected) model)))))

(fn request [db agent cofx]
  (let [selected (assert (selected-model db)
                         "selected model became unavailable")]
    (var (options problem) (values {} nil))
    (when (and misa.request-options misa.request-options.prepare)
      (set (options problem) (misa.request-options.prepare db selected)))
    (if problem (values agent nil problem)
        (let [sequence (+ agent.request_seq 1)
              id (.. :agent- sequence)]
          (values (misa.patch agent
                              {:request_seq sequence
                               :active_request_id id
                               :status :working
                               :cancel_requested false
                               :request_model selected.id
                               :request_started_wall_ms (and cofx cofx.clock
                                                             cofx.clock.wall_ms)
                               :request_started_monotonic_ms (and cofx
                                                                  cofx.clock
                                                                  cofx.clock.monotonic_ms)})
                  {: id
                   :messages agent.messages
                   :model selected.model
                   :request_options options
                   :system_prompt agent.system_prompt
                   :tools (misa.tools.all)
                   :type (.. :provider. selected.provider)})))))

(fn blocked [agent problem]
  (values (misa.patch agent {:status :ready :active_request_id misa.delete})
          [{:event {:level :error
                    : problem
                    :text problem.message
                    :type :transcript/harness}
            :type :dispatch}
           {:event {:status :ready :type :agent/status} :type :dispatch}
           {:event {:exit agent.exit_after_response :type :agent/completed}
            :type :dispatch}]))

(fn record-usage [agent usage]
  (if (= usage nil) agent
      (do
        (assert (= (type usage) :table) "usage must be a table")
        (let [normalized {}
              totals {}]
          (each [_ name (ipairs [:input_tokens
                                 :output_tokens
                                 :cache_read_tokens
                                 :cache_write_tokens])]
            (let [value (or (. usage name) 0)]
              (assert (and (= (type value) :number) (>= value 0)
                           (= (% value 1) 0))
                      (.. name " must be a nonnegative integer"))
              (tset normalized name value)
              (tset totals name (+ (. agent.usage name) value))))
          (tset normalized :input_includes_cache usage.input_includes_cache)
          (when (and (= (type usage.cost_usd) :number) (>= usage.cost_usd 0))
            (tset normalized :cost_usd usage.cost_usd))
          (misa.patch agent
                      {:usage totals :last_usage (misa.replace normalized)})))))

(fn tool-result [call-id text is-error]
  {:content [{:text (tostring text) :type :text}]
   :is_error (= is-error true)
   :role :tool
   :tool_call_id call-id})

(fn flush-tool-results [agent]
  (let [batch (assert agent.tool_batch "tool result batch is missing")
        results (icollect [_ call-id (ipairs batch.order)]
                  (assert (. batch.results call-id)
                          (.. "tool result is missing: " call-id)))
        messages (if (= (length results) 0) (misa.replace agent.messages)
                     (misa.append-all results))]
    (misa.patch agent {: messages :tool_batch misa.delete})))

(fn normalize-tool-arguments [blocks]
  (let [failures {}
        normalized (icollect [_ block (ipairs blocks)]
                     (if (not= block.type :tool_call) block
                         (let [arguments (if block.arguments block.arguments
                                             (do
                                               (assert (= (type block.arguments_json)
                                                          :string)
                                                       "tool call arguments are missing")
                                               (assert (and misa.json
                                                            (= (type misa.json.decode)
                                                               :function))
                                                       "JSON extension is required for provider tool calls")
                                               (let [(ok decoded) (pcall misa.json.decode
                                                                         (if (= block.arguments_json
                                                                                "")
                                                                             "{}"
                                                                             block.arguments_json))]
                                                 (if (and ok
                                                          (= (type decoded)
                                                             :table))
                                                     decoded
                                                     (do
                                                       (tset failures block.id
                                                             (if ok
                                                                 "tool arguments must be a JSON object"
                                                                 (tostring decoded)))
                                                       {})))))]
                           (misa.patch block
                                       {:arguments (misa.replace arguments)
                                        :arguments_json misa.delete}))))]
    (values normalized failures)))

(fn complete-response [db previous id raw-blocks usage stop-reason cofx]
  (let [(blocks argument-failures) (normalize-tool-arguments raw-blocks)]
    (content blocks :assistant)
    (let [stream previous.stream
          canonical (icollect [_ block (ipairs blocks)]
                      (misa.patch block
                                  {:transcript_id misa.delete
                                   :execution misa.delete}))]
      (var agent (misa.patch (record-usage previous usage)
                             {:messages (misa.append {:content canonical
                                                      :provider_state (and stream
                                                                           stream.provider_state)
                                                      :role :assistant})
                              :active_request_id misa.delete
                              :accepted_request_id id
                              :stream misa.delete
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
                           :started_wall_ms agent.request_started_wall_ms
                           :started_monotonic_ms agent.request_started_monotonic_ms
                           :response_id id
                           :role :assistant
                           :type :transcript/response-start}
                   :type :dispatch})
            (each [index block (ipairs blocks)]
              (let [block-id (.. id "/" index)]
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
                       :type :dispatch})))))
      (tset effects (+ (length effects) 1)
            {:event {:cost_usd (or (and usage usage.cost_usd)
                                   (and stream stream.usage
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
      (tset effects (+ (length effects) 1)
            {:event {:type :agent/history-changed} :type :dispatch})
      (each [_ block (ipairs blocks)]
        (when (= block.type :tool_call)
          (if (= block.execution :provider)
              (do
                (var result
                     (and stream stream.tool_results
                          (. stream.tool_results block.id)))
                (set result (or result
                                (tool-result block.id
                                             "Provider did not report a tool result"
                                             true)))
                (set agent (misa.patch agent {:messages (misa.append result)}))
                ;; Stream observations already completed the visible call.
                (when (not (and stream stream.tool_results
                                (. stream.tool_results block.id)))
                  (tset effects (+ (length effects) 1)
                        {:event {:id block.id
                                 :is_error result.is_error
                                 :text (. result.content 1 :text)
                                 :type :transcript/tool-result}
                         :type :dispatch})))
              (do
                (set saw-tool true)
                (when (not agent.tool_batch)
                  (set agent
                       (misa.patch agent
                                   {:tool_batch (misa.replace {:order []
                                                               :results {}})})))
                (assert (not (. agent.tool_batch.results block.id))
                        "duplicate tool call id")
                (set agent
                     (misa.patch agent
                                 {:tool_batch {:order (misa.append block.id)}}))
                (let [tool (misa.tools.lookup block.name)]
                  (if tool
                      (do
                        (assert (not (. agent.pending_tools block.id))
                                "duplicate tool call id")
                        (set agent
                             (misa.patch agent
                                         {:pending_tools {block.id {:name block.name
                                                                    :request_id id}}
                                          :pending_tool_count (+ agent.pending_tool_count
                                                                 1)}))
                        (table.insert effects
                                      {:type :dispatch
                                       :event {:type :transcript/tool-start
                                               :id block.id}})
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
                        (set agent
                             (misa.patch agent
                                         {:tool_batch {:results {block.id (tool-result block.id
                                                                                       message
                                                                                       true)}}}))
                        (tset effects (+ (length effects) 1)
                              {:event {:id block.id
                                       :is_error true
                                       :text message
                                       :type :transcript/tool-result}
                               :type :dispatch}))))))))
      (if (> agent.pending_tool_count 0)
          (do
            (set agent (misa.patch agent {:status :tools}))
            (tset effects (+ (length effects) 1)
                  {:event {:status :tools :type :agent/status} :type :dispatch}))
          saw-tool
          (do
            (set agent (flush-tool-results agent))
            (let [(next provider problem) (request db agent cofx)]
              (set agent next)
              (if problem
                  (let [(ready blocked-fx) (blocked agent problem)]
                    (set agent ready)
                    (each [_ effect (ipairs blocked-fx)]
                      (table.insert effects effect)))
                  (do
                    (tset effects (+ (length effects) 1)
                          {:event {:status :working :type :agent/status}
                           :type :dispatch})
                    (tset effects (+ (length effects) 1) provider)))))
          (do
            (set agent (misa.patch agent {:status :ready}))
            (tset effects (+ (length effects) 1)
                  {:event {:status :ready :type :agent/status} :type :dispatch})
            (tset effects (+ (length effects) 1)
                  {:event {:exit agent.exit_after_response
                           : id
                           :type :agent/completed}
                   :type :dispatch})))
      {:patch {:agent (misa.replace agent)} :fx effects})))

(fn empty-usage []
  {:cache_read_tokens 0 :cache_write_tokens 0 :input_tokens 0 :output_tokens 0})

(fn pending-ids [agent]
  (let [ids (icollect [id (pairs agent.pending_tools)] id)]
    (table.sort ids)
    ids))

(fn cancelled [_ agent id interrupt-response]
  (let [response-id (or id agent.active_request_id agent.accepted_request_id)
        messages (icollect [_ message (ipairs agent.messages)] message)]
    (when agent.stream
      (let [partial-content (icollect [_ block (ipairs agent.stream.blocks)]
                              (when (and (= block.type :text)
                                         (> (length block.chunks) 0))
                                {:text (table.concat block.chunks) :type :text}))]
        (when (> (length partial-content) 0)
          (table.insert messages {:content partial-content :role :assistant}))))
    (when agent.tool_batch
      (each [_ call-id (ipairs agent.tool_batch.order)]
        (table.insert messages
                      (or (. agent.tool_batch.results call-id)
                          (tool-result call-id :Cancelled true)))))
    (let [effects {}]
      (when (and interrupt-response response-id)
        (tset effects (+ (length effects) 1)
              {:event {:response_id response-id
                       :type :transcript/response-interrupted}
               :type :dispatch}))
      (tset effects (+ (length effects) 1)
            {:event {:level :warning
                     :text :Cancelled
                     :type :transcript/harness}
             :type :dispatch})
      (tset effects (+ (length effects) 1)
            {:event {:status :ready :type :agent/status} :type :dispatch})
      (tset effects (+ (length effects) 1)
            {:event {:type :agent/history-changed} :type :dispatch})
      (tset effects (+ (length effects) 1)
            {:event {:exit agent.exit_after_response
                     :id response-id
                     :type :agent/completed}
             :type :dispatch})
      {:patch {:agent {:messages (misa.replace messages)
                       :error misa.delete
                       :status :ready
                       :active_request_id misa.delete
                       :stream misa.delete
                       :cancel_requested false
                       :pending_tools (misa.replace {})
                       :pending_tool_count 0
                       :tool_batch misa.delete}}
       :fx effects})))

(fn complete-tool? [source]
  (or (not= source.type :tool_call)
      (and (= (type source.id) :string) (not= source.id "")
           (= (type source.name) :string) (not= source.name ""))))

(fn stream-blocks [stream]
  (when (accumulate [complete true _ source (ipairs stream.blocks)]
          (and complete (complete-tool? source)))
    (icollect [_ source (ipairs stream.blocks)]
      (let [block {:execution source.execution
                   :transcript_id source.transcript_id
                   :type source.type}]
        (if (or (= source.type :text) (= source.type :thinking))
            (tset block :text (table.concat source.chunks))
            (do
              (tset block :id source.id)
              (tset block :name source.name)
              (tset block :arguments source.arguments)
              (tset block :arguments_json
                    (if source.arguments_json_chunks
                        (table.concat source.arguments_json_chunks)
                        source.arguments_json))))
        block))))

(fn transcript-event [id kind fields]
  {:type :dispatch
   :event (misa.patch (or fields {}) {:type kind :response_id id})})

(fn stream-block [stream block kind id]
  (let [sequence (+ stream.block_seq 1)
        next-block (misa.patch block {:transcript_id (.. id "/" sequence)})]
    (values (misa.patch stream
                        {:block_seq sequence :blocks (misa.append next-block)})
            next-block (transcript-event id :transcript/block-start
                                        {:block_id next-block.transcript_id
                                         :call_id next-block.id
                                         :name next-block.name
                                         : kind}))))

(fn replaced-block [stream index block]
  (icollect [i previous (ipairs stream.blocks)] (if (= i index) block previous)))

(fn text-delta [previous delta id]
  "Append streamed text and describe its transcript update."
  (assert (= (type delta.text) :string) "stream text delta must be a string")
  (when (not= delta.text "")
    (var stream previous)
    (var block (. stream.blocks (length stream.blocks)))
    (let [fx []]
      (when (or (not block) (not= block.type delta.type))
        (let [(next created effect) (stream-block stream
                                                  {:type delta.type :chunks []}
                                                  (if (= delta.type :text)
                                                      :assistant
                                                      :thinking)
                                                  id)]
          (set stream next)
          (set block created)
          (table.insert fx effect)))
      (let [next-block (misa.patch block {:chunks (misa.append delta.text)})]
        (table.insert fx
                      (transcript-event id :transcript/block-delta
                                        {:block_id block.transcript_id
                                         :text delta.text}))
        {:patch {:block_seq stream.block_seq
                 :blocks (misa.replace (replaced-block stream
                                                       (length stream.blocks)
                                                       next-block))}
         : fx}))))

(fn tool-delta [previous delta id]
  "Accumulate a streamed tool invocation."
  (let [key (tostring (or delta.index delta.id (+ (length previous.blocks) 1)))]
    (var stream previous)
    (var index (. stream.tools key))
    (var block (and index (. stream.blocks index)))
    (let [fx []]
      (when (not block)
        (let [(next created effect) (stream-block stream
                                                  {:type :tool_call
                                                   :id delta.id
                                                   :name delta.name
                                                   :arguments_json_chunks []}
                                                  :tool_call id)]
          (set stream next)
          (set block created)
          (set index (length stream.blocks))
          (table.insert fx effect)))
      (let [next-block (misa.patch block
                                   {:execution delta.execution
                                    :id delta.id
                                    :name delta.name
                                    :arguments (when (not= delta.arguments nil)
                                                 (misa.replace delta.arguments))
                                    ;; A replacement resets the chunks, a delta
                                    ;; appends, and metadata alone leaves them.
                                    :arguments_json_chunks (if (not= delta.arguments
                                                                     nil)
                                                               (misa.replace nil)
                                                               (not= delta.arguments_json_delta
                                                                     nil)
                                                               (misa.append delta.arguments_json_delta)
                                                               (not= delta.arguments_json
                                                                     nil)
                                                               (misa.replace [delta.arguments_json])
                                                               (misa.replace block.arguments_json_chunks))})]
        (table.insert fx
                      (transcript-event id :transcript/block-delta
                                        {:block_id block.transcript_id
                                         :call_id delta.id
                                         :name delta.name
                                         :arguments delta.arguments
                                         :arguments_json delta.arguments_json
                                         :arguments_json_delta delta.arguments_json_delta}))
        {:patch {:block_seq stream.block_seq
                 :tools {key index}
                 :blocks (misa.replace (replaced-block stream index next-block))}
         : fx}))))

(fn active-stream [db event]
  (let [agent db.agent
        stream (and agent agent.stream)]
    (when (and stream (not agent.cancel_requested)
               (= event.id agent.active_request_id) (= event.id stream.id))
      stream)))

(fn continue-startup [db]
  "Start the queued initial prompt once authentication and model state are ready."
  (let [agent db.agent]
    (when (and agent agent.startup_prompt
               (or (not db.auth_startup) db.auth_startup.ready)
               (not (and db.models db.models.selection_pending)))
      {:patch {:agent {:startup_prompt misa.delete
                       :startup_attachments misa.delete}}
       :fx [{:type :dispatch
             :event {:type :agent/submit
                     :prompt agent.startup_prompt
                     :attachments agent.startup_attachments}}]})))

(fn stream-error [db event]
  "Finish a failed request and release pending work."
  (let [agent db.agent]
    (if (or (not agent) (not= event.id agent.active_request_id)
            (and (not= agent.status :working) (not= agent.status :cancelling)))
        nil
        (if agent.cancel_requested
            (cancelled db agent event.id (not= agent.stream nil))
            (let [had-stream (not= agent.stream nil)
                  message (tostring (or event.message "provider failed"))
                  fx {}]
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
              {:patch {:agent {:error message
                               :status :ready
                               :active_request_id misa.delete
                               :stream misa.delete
                               :accepted_request_id event.id}}
               : fx})))))

(fn start [config db _ cofx]
  "Initialize the conversation from application settings."
  (assert (or (= config.system_prompt nil)
              (= (type config.system_prompt) :string))
          "config.agent.system_prompt must be a string")
  (let [has-prompt (> (length cofx.argv) 0)
        prompt (when has-prompt
                 (table.concat cofx.argv " "))]
    {:patch {:agent (misa.replace {:exit_after_response has-prompt
                                   :messages []
                                   :pending_tool_count 0
                                   :pending_tools {}
                                   :request_seq 0
                                   :status :ready
                                   :system_prompt config.system_prompt
                                   :startup_prompt prompt
                                   :usage (empty-usage)})}
     ;; Observe auth only after every app/start owner has run.
     :fx (if has-prompt
             [{:type :dispatch :event {:type :agent/startup}}]
             [])}))

(fn cancel-active [db]
  "Request cancellation of the active response or tool batch."
  (let [agent db.agent]
    (when (and agent (not agent.cancel_requested))
      (let [ids (if (and (= agent.status :working) agent.active_request_id)
                    [agent.active_request_id]
                    (and (= agent.status :tools) (> agent.pending_tool_count 0))
                    (pending-ids agent)
                    [])]
        (when (> (length ids) 0)
          (let [fx [{:type :dispatch
                     :event {:type :agent/status :status :cancelling}}]]
            (each [_ id (ipairs ids)]
              (table.insert fx {:type :operation/cancel : id}))
            {:patch {:agent {:cancel_requested true :status :cancelling}} : fx}))))))

(fn conversation-loaded [db event]
  "Replace canonical history with a resumed conversation."
  (let [agent (assert db.agent "agent state is not initialized")]
    (assert (and (= (type event.messages) :table) (> (length event.messages) 0))
            "a resumed conversation needs messages")
    (assert (= agent.status :ready)
            "cannot resume a conversation while a request is active")
    {:patch {:agent (misa.patch agent
                                {:messages (misa.replace event.messages)
                                 :active_request_id misa.delete
                                 :accepted_request_id misa.delete
                                 :pending_tool_count 0
                                 :pending_tools (misa.replace {})
                                 :stream misa.delete
                                 :tool_batch misa.delete
                                 :error misa.delete})}
     :fx [{:type :dispatch :event {:type :agent/history-changed}}]}))

(fn reset [db]
  "Clear conversation state and cancel outstanding operations."
  (let [agent (assert db.agent "agent state is not initialized")
        fx []]
    (when agent.active_request_id
      (table.insert fx {:type :operation/cancel :id agent.active_request_id}))
    (each [_ id (ipairs (pending-ids agent))]
      (table.insert fx {:type :operation/cancel : id}))
    (table.insert fx {:type :dispatch :event {:type :transcript/reset}})
    (table.insert fx {:type :dispatch :event {:type :agent/history-changed}})
    (table.insert fx {:type :dispatch
                      :event {:type :agent/status
                              :status :ready
                              :last_usage {}
                              :usage (empty-usage)}})
    {:patch {:agent {:status :ready
                     :active_request_id misa.delete
                     :accepted_request_id misa.delete
                     :stream misa.delete
                     :cancel_requested false
                     :pending_tools (misa.replace {})
                     :pending_tool_count 0
                     :tool_batch misa.delete
                     :messages (misa.replace [])
                     :error misa.delete
                     :startup_prompt misa.delete
                     :startup_attachments misa.delete
                     :last_usage (misa.replace {})
                     :usage (misa.replace (empty-usage))}}
     : fx}))

(fn submit [db event cofx]
  "Start a user turn or queue it behind active work."
  (assert (and (= (type event.prompt) :string)
               (or (not= event.prompt "")
                   (> (length (or event.attachments [])) 0)))
          "agent prompt must be nonempty")
  (let [agent (assert db.agent "agent state is not initialized")]
    (if (not= agent.status :ready) nil
        (or (and db.auth_startup (not db.auth_startup.ready))
            (and db.models db.models.selection_pending))
        {:patch {:agent {:startup_prompt event.prompt
                         :startup_attachments (misa.replace event.attachments)}}}
        (not (selected-model db))
        (let [chosen (or (and db.models db.models.preferred)
                         (and db.models db.models.configured_default))
              message (if chosen
                          (.. "model is unavailable: " chosen)
                          "no available models; log in to a provider")
              problem {:code :missing_model
                       :kind :request_readiness
                       : message
                       :model chosen}]
          {:fx [{:type :dispatch
                 :event {:type :transcript/harness
                         :level :error
                         : problem
                         :text message}}
                {:type :dispatch
                 :event {:type :agent/completed
                         :exit agent.exit_after_response}}]})
        (let [blocks (if (= event.prompt "")
                         []
                         [{:type :text :text event.prompt}])]
          (each [_ image (ipairs (or event.attachments []))]
            (assert (and (= image.type :image) (= (type image.source) :table)
                         (= image.source.type :base64)
                         (= (type image.source.media_type) :string)
                         (= (type image.source.data) :string))
                    "invalid image attachment")
            (table.insert blocks image))
          (let [history (misa.patch agent
                                    {:messages (misa.append {:content blocks
                                                             :role :user})})
                (next provider problem) (request db history cofx)]
            (if problem
                (let [(ready fx) (blocked agent problem)]
                  {:patch {:agent (misa.replace ready)} : fx})
                {:patch {:agent (misa.replace next)}
                 :fx [{:type :dispatch :event {:type :agent/history-changed}}
                      {:type :dispatch
                       :event {:type :agent/submitted :prompt event.prompt}}
                      {:type :dispatch
                       :event {:type :transcript/user
                               :text event.prompt
                               :attachments event.attachments}}
                      {:type :dispatch
                       :event {:type :agent/status :status :working}}
                      provider]}))))))

(fn stream-start [db event]
  "Initialize correlated response assembly."
  (let [agent db.agent]
    (when (and agent (= agent.status :working)
               (= event.id agent.active_request_id) (not agent.stream))
      {:patch {:agent {:stream (misa.replace {:id event.id
                                              :block_seq 0
                                              :blocks []
                                              :tools {}
                                              :tool_results {}})}}
       :fx [(transcript-event event.id :transcript/response-start
                              {:model agent.request_model
                               :role :assistant
                               :started_wall_ms agent.request_started_wall_ms
                               :started_monotonic_ms agent.request_started_monotonic_ms})]})))

(fn stream-delta [db event]
  "Apply a correlated response delta."
  (let [stream (active-stream db event)]
    (when (and stream (= (type event.delta) :table))
      (let [handler (assert (. (misa.catalog :agent-deltas) event.delta.type)
                            (.. "unsupported agent stream delta: "
                                (tostring event.delta.type)))
            result (handler stream event.delta event.id)]
        (when result
          {:patch {:agent {:stream (or result.patch {})}} :fx result.fx})))))

(fn stream-tool-result [db event]
  "Record a provider-owned tool result."
  (when (active-stream db event)
    (assert (= (type event.tool_call_id) :string)
            "provider tool result requires a call id")
    {:patch {:agent {:stream {:tool_results {event.tool_call_id (misa.replace (tool-result event.tool_call_id
                                                                                           (or event.text
                                                                                               "")
                                                                                           event.is_error))}}}}
     :fx [{:type :dispatch
           :event {:type :transcript/tool-result
                   :id event.tool_call_id
                   :is_error (= event.is_error true)
                   :text (or event.text "")}}]}))

(fn stream-usage [db event]
  "Capture token usage for the active response."
  (when (active-stream db event)
    (let [value (if (= (type event.usage) :table)
                    event.usage
                    {})]
      {:patch {:agent {:stream {:stop_reason event.stop_reason
                                :usage {:input_tokens value.input_tokens
                                        :output_tokens value.output_tokens
                                        :cache_read_tokens value.cache_read_tokens
                                        :cache_write_tokens value.cache_write_tokens
                                        :cost_usd value.cost_usd
                                        :input_includes_cache value.input_includes_cache}}}}})))

(fn stream-state [db event]
  "Capture opaque continuation state for the active response."
  (let [stream (active-stream db event)]
    (when stream
      (assert (and (= (type event.provider) :string)
                   (= (type event.value) :table))
              "invalid provider continuation state")
      {:patch {:agent {:stream {:provider_state (misa.append {:provider event.provider
                                                              :value event.value})}}}})))

(fn stream-end [db event cofx]
  "Complete response assembly and continue the conversation."
  (let [agent db.agent
        stream (and agent agent.stream)]
    (if (or (not stream) (not= event.id agent.active_request_id)
            (not= stream.id event.id))
        nil
        (if agent.cancel_requested
            (cancelled db agent event.id true)
            (if (not= agent.status :working)
                nil
                (let [blocks (stream-blocks stream)]
                  (if (not blocks)
                      {:fx [{:event {:id event.id
                                     :message "provider ended an incomplete tool call"
                                     :type :agent/stream-error}
                             :type :dispatch}]}
                      (complete-response db agent event.id blocks
                                         (or event.usage stream.usage)
                                         (or event.stop_reason
                                             stream.stop_reason)
                                         cofx))))))))

(fn legacy-result [db event cofx]
  "Normalize a complete provider result into the conversation."
  (let [agent db.agent]
    (if (or (not agent) (not= event.id agent.active_request_id))
        nil
        (if agent.cancel_requested
            (cancelled db agent event.id (not= agent.stream nil))
            (if (not= agent.status :working)
                nil
                (complete-response db agent event.id event.content event.usage
                                   event.stop_reason cofx))))))

(fn receive-tool-result [db event cofx]
  "Collect a local tool result and continue when its batch completes."
  (let [previous db.agent]
    (when (and previous (. previous.pending_tools event.tool_call_id))
      (var agent (misa.patch previous
                             {:pending_tools {event.tool_call_id misa.delete}
                              :pending_tool_count (- previous.pending_tool_count
                                                     1)}))
      (let [update {:type :dispatch
                    :event {:type :transcript/tool-result
                            :id event.tool_call_id
                            :cancelled (when agent.cancel_requested true)
                            :is_error (= event.is_error true)
                            :text (or event.text
                                      (if agent.cancel_requested :Cancelled
                                          ""))}}]
        (if agent.cancel_requested
            (if (= agent.pending_tool_count 0)
                (let [result (cancelled db agent agent.accepted_request_id true)]
                  (table.insert result.fx 1 update)
                  result)
                {:patch {:agent (misa.replace agent)} :fx [update]})
            (do
              (assert (and agent.tool_batch
                           (not (. agent.tool_batch.results event.tool_call_id)))
                      "duplicate tool result")
              (set agent
                   (misa.patch agent
                               {:tool_batch {:results {event.tool_call_id (tool-result event.tool_call_id
                                                                                       (or event.text
                                                                                           "")
                                                                                       event.is_error)}}}))
              (let [fx [update]]
                (when (= agent.pending_tool_count 0)
                  (set agent (flush-tool-results agent))
                  (tset fx (+ (length fx) 1)
                        {:type :dispatch :event {:type :agent/history-changed}})
                  (let [(next provider problem) (request db agent cofx)]
                    (set agent next)
                    (if problem
                        (let [(ready blocked-fx) (blocked agent problem)]
                          (set agent ready)
                          (each [_ effect (ipairs blocked-fx)]
                            (table.insert fx effect)))
                        (do
                          (table.insert fx
                                        {:type :dispatch
                                         :event {:type :agent/status
                                                 :status :working}})
                          (table.insert fx provider)))))
                {:patch {:agent (misa.replace agent)} : fx})))))))

{:cancel cancel-active
 : conversation-loaded
 : start
 : continue-startup
 : reset
 : submit
 : stream-start
 : stream-delta
 : stream-tool-result
 : stream-usage
 : stream-state
 : stream-end
 : legacy-result
 : receive-tool-result
 : stream-error
 : text-delta
 : tool-delta}
