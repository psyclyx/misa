;; ChatGPT subscription provider using the Codex Responses SSE protocol.

(fn input [messages]
  (let [result {}]
    (each [_ message (ipairs messages)]
      (if (= message.role :tool)
          (do
            (var text "")
            (each [_ block (ipairs (or message.content {}))]
              (when (= block.type :text) (set text (.. text block.text))))
            (tset result (+ (length result) 1)
                  {:call_id message.tool_call_id
                   :output text
                   :type :function_call_output}))
          (do
            (each [_ state (ipairs (or message.provider_state {}))]
              (when (= state.provider :openai-codex)
                (tset result (+ (length result) 1) state.value)))
            (local content {})
            (each [_ block (ipairs (or message.content {}))]
              (if (= block.type :text)
                  (tset content (+ (length content) 1)
                        {:text block.text
                         :type (or (and (= message.role :assistant)
                                        :output_text)
                                   :input_text)})
                  (= block.type :image)
                  (tset content (+ (length content) 1)
                        {:image_url (.. "data:" block.source.media_type
                                        ";base64," block.source.data)
                         :type :input_image})
                  (= block.type :tool_call)
                  (tset result (+ (length result) 1)
                        {:arguments ((. (assert misa.json
                                                "provider.openai-codex requires json for tool history")
                                        :encode) block.arguments)
                         :call_id block.id
                         :name block.name
                         :type :function_call})))
            (when (> (length content) 0)
              (tset result (+ (length result) 1) {: content :role message.role})))))
    result))

(fn tools [___values___]
  (let [result {}]
    (each [_ tool (ipairs ___values___)]
      (tset result (+ (length result) 1)
            {:description tool.description
             :name tool.name
             :parameters tool.input_schema
             :strict false
             :type :function}))
    result))

{:setup (fn [context]
          (misa.reg_auth_provider {:description "ChatGPT subscription OAuth"
                                   :id :openai-codex
                                   :label "OpenAI Codex"
                                   :model_provider :openai-codex
                                   :profile {:authorization_url "https://auth.openai.com/api/accounts/deviceauth/usercode"
                                             :id :default
                                             :token_url "https://auth.openai.com/oauth/token"}
                                   :strategy :device_oauth})
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table)
                               providers.openai_codex)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local serializer-id :openai.responses.codex)
          (misa.reg_request_options_serializer serializer-id
                                               {:accepts (fn [name]
                                                           (= name
                                                              :reasoning_effort))
                                                :serialize (fn [body
                                                                name
                                                                value]
                                                             (if (not= name
                                                                       :reasoning_effort)
                                                                 false
                                                                 (do
                                                                   (set body.reasoning
                                                                        {:effort value
                                                                         :summary :auto})
                                                                   true)))})
          (local reasoning-api
                 {:request_options {:reasoning_effort {:choices [:low
                                                                 :medium
                                                                 :high
                                                                 :xhigh]
                                                       :default :medium}}
                  :request_options_serializer serializer-id})
          (each [_ model (ipairs (or config.models
                                     [{:api reasoning-api
                                       :context_window 1000000
                                       :id :openai-codex/gpt-5.4
                                       :label "GPT-5.4 (ChatGPT)"
                                       :model :gpt-5.4}
                                      {:api reasoning-api
                                       :context_window 400000
                                       :id :openai-codex/gpt-5.3-codex
                                       :label "GPT-5.3 Codex"
                                       :model :gpt-5.3-codex}]))]
            (var api model.api)
            (when (and (= (type api) :table)
                       (= (type api.request_options) :table))
              (local copy {})
              (each [key value (pairs api)] (tset copy key value))
              (set copy.request_options_serializer serializer-id)
              (set api copy))
            (misa.reg_model {: api
                             :context_window model.context_window
                             :id model.id
                             :label (or model.label model.id)
                             :model model.model
                             :provider :openai-codex}))
          (misa.reg_fx :provider.openai-codex
                       (fn [effect]
                         (local body
                                {:include [:reasoning.encrypted_content]
                                 :input (input effect.messages)
                                 :instructions (or effect.system_prompt
                                                   "You are a helpful coding assistant.")
                                 :model effect.model
                                 :parallel_tool_calls true
                                 :reasoning {:summary :auto}
                                 :store false
                                 :stream true
                                 :text {:verbosity :low}
                                 :tool_choice :auto})
                         (misa.serialize_request_options serializer-id
                                                         (or effect.request_options
                                                             {})
                                                         body)
                         (local definitions (tools effect.tools))
                         (when (> (length definitions) 0)
                           (set body.tools definitions))
                         {:completion :provider/openai-codex-complete
                          :credential {:header :authorization
                                       :id :openai-codex
                                       :metadata_field :account_id
                                       :metadata_header :chatgpt-account-id
                                       :prefix "Bearer "}
                          :headers [{:name :content-type
                                     :value :application/json}
                                    {:name :accept :value :text/event-stream}
                                    {:name :openai-beta
                                     :value :responses=experimental}
                                    {:name :originator :value :misa}
                                    {:name :user-agent :value :misa/0.1}]
                          :id effect.id
                          :json body
                          :method :POST
                          :response_format :sse_json_stream
                          :timeouts config.timeouts
                          :type :http/request
                          :url (or config.url
                                   "https://chatgpt.com/backend-api/codex/responses")}))
          (misa.reg_event :provider/openai-codex-complete
                          (fn [db event]
                            (set db.providers (or db.providers {}))
                            (set db.providers.codex_streams
                                 (or db.providers.codex_streams {}))
                            (local streams db.providers.codex_streams)
                            (if (= event.phase :start)
                                (do
                                  (tset streams event.id {:reasoning {}})
                                  {: db
                                   :fx [{:event {:id event.id
                                                 :type :agent/stream-start}
                                         :type :dispatch}]})
                                (do
                                  (local state
                                         (or (. streams event.id)
                                             {:reasoning {}}))
                                  (if (= event.phase :end)
                                      (do
                                        (local terminal (= state.terminal true))
                                        (tset streams event.id nil)
                                        (local next-event
                                               (or (and (and event.ok terminal)
                                                        {:id event.id
                                                         :type :agent/stream-end})
                                                   {:id event.id
                                                    :message (or (or (or event.message
                                                                         (and (not terminal)
                                                                              "Codex stream ended without response.completed"))
                                                                     event.body)
                                                                 (.. "HTTP "
                                                                     (tostring event.status)))
                                                    :type :agent/stream-error}))
                                        {: db
                                         :fx [{:event next-event
                                               :type :dispatch}]})
                                      (do
                                        (var (fx terminal) (values {} false))
                                        (each [_ record (ipairs (or event.records
                                                                    {}))]
                                          (if (and (= record.type
                                                      :response.output_text.delta)
                                                   (= (type record.delta)
                                                      :string))
                                              (tset fx (+ (length fx) 1)
                                                    {:event {:delta {:text record.delta
                                                                     :type :text}
                                                             :id event.id
                                                             :type :agent/stream-delta}
                                                     :type :dispatch})
                                              (and (or (= record.type
                                                          :response.reasoning_summary_text.delta)
                                                       (= record.type
                                                          :response.reasoning_text.delta))
                                                   (= (type record.delta)
                                                      :string))
                                              (do
                                                (tset state.reasoning
                                                      (tostring (or (or record.output_index
                                                                        record.item_id)
                                                                    :current))
                                                      true)
                                                (tset fx (+ (length fx) 1)
                                                      {:event {:delta {:text record.delta
                                                                       :type :thinking}
                                                               :id event.id
                                                               :type :agent/stream-delta}
                                                       :type :dispatch}))
                                              (and (and (= record.type
                                                           :response.output_item.added)
                                                        (= (type record.item)
                                                           :table))
                                                   (= record.item.type
                                                      :function_call))
                                              (tset fx (+ (length fx) 1)
                                                    {:event {:delta {:arguments_json (or record.item.arguments
                                                                                         "")
                                                                     :id record.item.call_id
                                                                     :index record.output_index
                                                                     :name record.item.name
                                                                     :type :tool_call}
                                                             :id event.id
                                                             :type :agent/stream-delta}
                                                     :type :dispatch})
                                              (and (= record.type
                                                      :response.function_call_arguments.delta)
                                                   (= (type record.delta)
                                                      :string))
                                              (tset fx (+ (length fx) 1)
                                                    {:event {:delta {:arguments_json_delta record.delta
                                                                     :index record.output_index
                                                                     :type :tool_call}
                                                             :id event.id
                                                             :type :agent/stream-delta}
                                                     :type :dispatch})
                                              (and (and (= record.type
                                                           :response.output_item.done)
                                                        (= (type record.item)
                                                           :table))
                                                   (= record.item.type
                                                      :reasoning))
                                              (do
                                                (tset fx (+ (length fx) 1)
                                                      {:event {:id event.id
                                                               :provider :openai-codex
                                                               :type :agent/stream-state
                                                               :value record.item}
                                                       :type :dispatch})
                                                (when (not (. state.reasoning
                                                              (tostring (or (or record.output_index
                                                                                record.item.id)
                                                                            :current))))
                                                  (each [_ summary (ipairs (or record.item.summary
                                                                               {}))]
                                                    (when (and (= (type summary.text)
                                                                  :string)
                                                               (not= summary.text
                                                                     ""))
                                                      (tset fx
                                                            (+ (length fx) 1)
                                                            {:event {:delta {:text summary.text
                                                                             :type :thinking}
                                                                     :id event.id
                                                                     :type :agent/stream-delta}
                                                             :type :dispatch})))))
                                              (and (and (= record.type
                                                           :response.output_item.done)
                                                        (= (type record.item)
                                                           :table))
                                                   (= record.item.type
                                                      :function_call))
                                              (tset fx (+ (length fx) 1)
                                                    {:event {:delta {:arguments_json (or record.item.arguments
                                                                                         "{}")
                                                                     :id record.item.call_id
                                                                     :index record.output_index
                                                                     :name record.item.name
                                                                     :type :tool_call}
                                                             :id event.id
                                                             :type :agent/stream-delta}
                                                     :type :dispatch})
                                              (and (= record.type
                                                      :response.completed)
                                                   (= (type record.response)
                                                      :table))
                                              (do
                                                (set (state.terminal terminal)
                                                     (values true true))
                                                (local usage
                                                       (or (and (= (type record.response.usage)
                                                                   :table)
                                                                record.response.usage)
                                                           {}))
                                                (local details
                                                       (or (and (= (type usage.input_tokens_details)
                                                                   :table)
                                                                usage.input_tokens_details)
                                                           {}))
                                                (tset fx (+ (length fx) 1)
                                                      {:event {:id event.id
                                                               :type :agent/stream-usage
                                                               :usage {:cache_read_tokens (or details.cached_tokens
                                                                                              0)
                                                                       :cache_write_tokens 0
                                                                       :cost_usd usage.cost
                                                                       :input_includes_cache true
                                                                       :input_tokens (or usage.input_tokens
                                                                                         0)
                                                                       :output_tokens (or usage.output_tokens
                                                                                          0)}}
                                                       :type :dispatch})
                                                (lua :break))
                                              (or (or (= record.type :error)
                                                      (= record.type
                                                         :response.failed))
                                                  (= record.type
                                                     :response.incomplete))
                                              (do
                                                (tset fx (+ (length fx) 1)
                                                      {:event {:id event.id
                                                               :message (tostring (or (or record.message
                                                                                          (and (and record.response
                                                                                                    record.response.error)
                                                                                               record.response.error.message))
                                                                                      "Codex request did not complete"))
                                                               :type :agent/stream-error}
                                                       :type :dispatch})
                                                (set terminal true)
                                                (lua :break))))
                                        (tset streams event.id state)
                                        (when terminal
                                          (tset fx (+ (length fx) 1)
                                                {:id event.id
                                                 :type :operation/finish}))
                                        {: db : fx}))))))
          nil)}

