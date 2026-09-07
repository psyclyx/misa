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


(fn delta [id value]
  {:type :dispatch :event {:type :agent/stream-delta : id :delta value}})

(fn text-record [kind]
  (fn [_ record id]
    (when (= (type record.delta) :string)
      {:fx [(delta id {:type kind :text record.delta})]})))

(fn reasoning-record [_ record id]
  (when (= (type record.delta) :string)
    {:patch {:reasoning {(tostring (or record.output_index record.item_id :current)) true}}
     :fx [(delta id {:type :thinking :text record.delta})]}))

(fn function-record [record id fallback]
  {:fx [(delta id {:type :tool_call :arguments_json (or record.item.arguments fallback)
                  :id record.item.call_id :index record.output_index :name record.item.name})]})

(fn done-record [state record id]
  (when (= (type record.item) :table)
    (if (= record.item.type :function_call) (function-record record id "{}")
        (= record.item.type :reasoning)
        (let [fx [{:type :dispatch :event {:type :agent/stream-state : id
                                         :provider :openai-codex :value record.item}}]
              key (tostring (or record.output_index record.item.id :current))]
          (when (not (. state.reasoning key))
            (each [_ summary (ipairs (or record.item.summary {}))]
              (when (and (= (type summary.text) :string) (not= summary.text ""))
                (table.insert fx (delta id {:type :thinking :text summary.text})))))
          {:patch {:reasoning {key true}} : fx}))))

(fn completed-record [_ record id]
  (when (= (type record.response) :table)
    (local usage (if (= (type record.response.usage) :table) record.response.usage {}))
    (local details (if (= (type usage.input_tokens_details) :table) usage.input_tokens_details {}))
    {:patch {:terminal true} :finish true
     :fx [{:type :dispatch :event {:type :agent/stream-usage : id
                                   :usage {:cache_read_tokens (or details.cached_tokens 0)
                                           :cache_write_tokens 0 :cost_usd usage.cost
                                           :input_includes_cache true
                                           :input_tokens (or usage.input_tokens 0)
                                           :output_tokens (or usage.output_tokens 0)}}}]}))

(fn failed-record [_ record id]
  {:patch {:failed true} :finish true
   :fx [{:type :dispatch
         :event {:type :agent/stream-error : id
                 :message (tostring (or record.message
                                        (and record.response record.response.error record.response.error.message)
                                        "Codex request did not complete"))}}]})

(local records
       {:response.output_text.delta (text-record :text)
        :response.reasoning_summary_text.delta reasoning-record
        :response.reasoning_text.delta reasoning-record
        :response.output_item.added (fn [_ record id]
                                     (when (and (= (type record.item) :table) (= record.item.type :function_call))
                                       (function-record record id "")))
        :response.function_call_arguments.delta
        (fn [_ record id]
          (when (= (type record.delta) :string)
            {:fx [(delta id {:type :tool_call :arguments_json_delta record.delta
                            :index record.output_index})]}))
        :response.output_item.done done-record
        :response.completed completed-record
        :error failed-record :response.failed failed-record :response.incomplete failed-record})

(fn stream-update [id state fx]
  {:patch {:providers {:codex_streams {id (misa.replace state)}}} : fx})

(fn stream [db event]
  (local streams (or (and db.providers db.providers.codex_streams) {}))
  (local state (or (. streams event.id) {:reasoning {}}))
  (if (= event.phase :start)
      (stream-update event.id {:reasoning {}}
                     [{:type :dispatch :event {:type :agent/stream-start :id event.id}}])
      (= event.phase :end)
      (let [successful (and event.ok state.terminal)
            next-event (if successful {:id event.id :type :agent/stream-end}
                           {:id event.id :type :agent/stream-error
                            :message (or event.message
                                         (when (not state.terminal) "Codex stream ended without response.completed")
                                         event.body (.. "HTTP " (tostring event.status)))})]
        (stream-update event.id nil (if state.failed [] [{:type :dispatch :event next-event}])))
      (or state.terminal state.failed) nil
      (do
        (var next-state state)
        (local fx [])
        (var finished false)
        (each [_ record (ipairs (or event.records {})) &until finished]
          (local handler (. records record.type))
          (local result (and handler (handler next-state record event.id)))
          (when result
            (set next-state (misa.patch next-state (or result.patch {})))
            (each [_ effect (ipairs (or result.fx []))] (table.insert fx effect))
            (when result.finish
              (set finished true)
              (table.insert fx {:id event.id :type :operation/finish}))))
        (stream-update event.id next-state fx))))

{:setup (fn [context]
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/auth-provider
                         :value {:description "ChatGPT subscription OAuth"
                                 :id :openai-codex
                                 :label "OpenAI Codex"
                                 :model_provider :openai-codex
                                 :profile {:authorization_url "https://auth.openai.com/api/accounts/deviceauth/usercode"
                                           :id :default
                                           :token_url "https://auth.openai.com/oauth/token"}
                                 :strategy :device_oauth}})
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table)
                               providers.openai_codex)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local serializer-id :openai.responses.codex)
          (table.insert setup-fx
                        {:type :register/request-options-serializer
                         :id serializer-id
                         :serializer {:accepts (fn [name]
                                                 (= name :reasoning_effort))
                                      :serialize (fn [body name value]
                                                   (if (not= name
                                                             :reasoning_effort)
                                                       false
                                                       (do
                                                         (set body.reasoning
                                                              {:effort value
                                                               :summary :auto})
                                                         true)))}})
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
            (local api (if (and (= (type model.api) :table)
                                (= (type model.api.request_options) :table))
                           (misa.patch model.api {:request_options_serializer serializer-id})
                           model.api))
            (table.insert setup-fx
                          {:type :register/model
                           :value {: api
                                   :context_window model.context_window
                                   :id model.id
                                   :label (or model.label model.id)
                                   :model model.model
                                   :provider :openai-codex}}))
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.openai-codex
                         :handler (fn [effect]
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
                                               {:name :accept
                                                :value :text/event-stream}
                                               {:name :openai-beta
                                                :value :responses=experimental}
                                               {:name :originator :value :misa}
                                               {:name :user-agent
                                                :value :misa/0.1}]
                                     :id effect.id
                                     :json body
                                     :method :POST
                                     :response_format :sse_json_stream
                                     :timeouts config.timeouts
                                     :type :http/request
                                     :url (or config.url
                                              "https://chatgpt.com/backend-api/codex/responses")})})
          (table.insert setup-fx
                        {:type :register/setup-effect :name :register/codex-record
                         :handler (fn [effect]
                                    (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                 (= (type effect.value) :function) (not (. records effect.id)))
                                            "invalid or duplicate Codex record handler")
                                    (tset records effect.id effect.value))})
          (table.insert setup-fx
                        {:type :register/event :name :provider/openai-codex-complete :handler stream})
          {:fx setup-fx})}
