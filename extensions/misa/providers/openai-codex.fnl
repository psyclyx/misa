(local definitions (require :misa.definitions))

;; ChatGPT subscription provider using the Codex Responses SSE protocol.

(fn quota-number [value]
  (let [number (tonumber value)]
    (when (and number (= number number) (>= number 0) (< number math.huge))
      number)))

(fn quota-label [value]
  (when (= (type value) :string) value))

(fn window-label [seconds fallback]
  (if (and seconds (> seconds 0))
      (let [unit (or (accumulate [found nil _ item (ipairs [[86400 :d]
                                                            [3600 :h]
                                                            [60 :m]])
                                  &until found]
                       (when (= (% seconds (. item 1)) 0)
                         item)) [1 :s])]
        (.. (/ seconds (. unit 1)) (. unit 2)))
      fallback))

(fn usage-windows [payload]
  (let [windows []]
    (fn add [bucket label]
      (when (= (type bucket) :table)
        (each [_ slot (ipairs [{:key :primary_window :label :primary}
                               {:key :secondary_window :label :secondary}])]
          (let [window (. bucket slot.key)]
            (when (= (type window) :table)
              (let [used (quota-number window.used_percent)]
                (when used
                  (let [seconds (quota-number window.limit_window_seconds)]
                    (table.insert windows
                                  {:label (.. label " · "
                                              (window-label seconds slot.label))
                                   :unit :percent
                                   :limit 100
                                   : used
                                   :remaining (math.max 0 (- 100 used))
                                   :window_seconds seconds
                                   :reset_at_unix (quota-number window.reset_at)
                                   :reset_after_seconds (quota-number window.reset_after_seconds)})))))))))

    (when (= (type payload) :table)
      (add payload.rate_limit "Codex")
      (add payload.code_review_rate_limit "Code review")
      (each [index item (ipairs (if (= (type payload.additional_rate_limits)
                                       :table)
                                    payload.additional_rate_limits
                                    []))]
        (when (= (type item) :table)
          (add item.rate_limit
               (or (quota-label item.limit_name)
                   (quota-label item.metered_feature)
                   (.. "Additional quota " index))))))
    windows))

(fn reset-count [payload]
  (let [summary (and (= (type payload) :table) payload.rate_limit_reset_credits)]
    (when (= (type summary) :table) (quota-number summary.available_count))))

;; The usage endpoint carries only a count. Detail rows come from the separate
;; read-only reset-credit endpoint; keep wire dates out of display code.
(fn reset-credits [payload]
  (when (and (= (type payload) :table) (= (type payload.credits) :table)
             (not= payload.credits misa.json-null))
    (icollect [_ credit (ipairs payload.credits)]
      (when (and (= (type credit) :table) (= (type credit.id) :string)
                 (= (type credit.status) :string)
                 (or (quota-label credit.granted_at)
                     (quota-number credit.granted_at)))
        {:id credit.id
         :label (or (quota-label credit.title) "Quota reset")
         :status (quota-label credit.status)
         :granted_at (quota-label credit.granted_at)
         :granted_at_unix (quota-number credit.granted_at)
         :expires_at (quota-label credit.expires_at)
         :expires_at_unix (quota-number credit.expires_at)
         :expires_never (or (= credit.expires_at nil)
                            (= credit.expires_at misa.json-null))}))))

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
            (let [content {}]
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
                (tset result (+ (length result) 1)
                      {: content :role message.role}))))))
    result))

(fn tools [items]
  (let [result {}]
    (each [_ tool (ipairs items)]
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
    {:patch {:reasoning {(tostring (or record.output_index record.item_id
                                        :current)) true}}
     :fx [(delta id {:type :thinking :text record.delta})]}))

(fn function-record [record id fallback]
  {:fx [(delta id {:type :tool_call
                   :arguments_json (or record.item.arguments fallback)
                   :id record.item.call_id
                   :index record.output_index
                   :name record.item.name})]})

(fn done-record [state record id]
  (when (= (type record.item) :table)
    (if (= record.item.type :function_call) (function-record record id "{}")
        (= record.item.type :reasoning)
        (let [fx [{:type :dispatch
                   :event {:type :agent/stream-state
                           : id
                           :provider :openai-codex
                           :value record.item}}]
              key (tostring (or record.output_index record.item.id :current))]
          (when (not (. state.reasoning key))
            (each [_ summary (ipairs (or record.item.summary {}))]
              (when (and (= (type summary.text) :string) (not= summary.text ""))
                (table.insert fx
                              (delta id {:type :thinking :text summary.text})))))
          {:patch {:reasoning {key true}} : fx}))))

(fn completed-record [_ record id]
  (when (= (type record.response) :table)
    (let [usage (if (= (type record.response.usage) :table)
                    record.response.usage
                    {})
          details (if (= (type usage.input_tokens_details) :table)
                      usage.input_tokens_details
                      {})]
      {:patch {:terminal true}
       :finish true
       :fx [{:type :dispatch
             :event {:type :agent/stream-usage
                     : id
                     :usage {:cache_read_tokens (or details.cached_tokens 0)
                             :cache_write_tokens 0
                             :cost_usd usage.cost
                             :input_includes_cache true
                             :input_tokens (or usage.input_tokens 0)
                             :output_tokens (or usage.output_tokens 0)}}}]})))

(fn failed-record [_ record id]
  {:patch {:failed true}
   :finish true
   :fx [{:type :dispatch
         :event {:type :agent/stream-error
                 : id
                 :message (tostring (or record.message
                                        (and record.response
                                             record.response.error
                                             record.response.error.message)
                                        "Codex request did not complete"))}}]})

(local records {:response.output_text.delta (text-record :text)
                :response.reasoning_summary_text.delta reasoning-record
                :response.reasoning_text.delta reasoning-record
                :response.output_item.added (fn [_ record id]
                                              (when (and (= (type record.item)
                                                            :table)
                                                         (= record.item.type
                                                            :function_call))
                                                (function-record record id "")))
                :response.function_call_arguments.delta (fn [_ record id]
                                                          (when (= (type record.delta)
                                                                   :string)
                                                            {:fx [(delta id
                                                                         {:type :tool_call
                                                                          :arguments_json_delta record.delta
                                                                          :index record.output_index})]}))
                :response.output_item.done done-record
                :response.completed completed-record
                :error failed-record
                :response.failed failed-record
                :response.incomplete failed-record})

(fn stream-update [id state fx]
  {:patch {:providers {:codex_streams {id (misa.replace state)}}}
   :fx (misa.stream.effects fx)})

(fn stream [db event]
  (let [streams (or (and db.providers db.providers.codex_streams) {})
        state (or (. streams event.id) {:reasoning {}})]
    (if (= event.phase :start)
        (stream-update event.id {:reasoning {}}
                       [{:type :dispatch
                         :event {:type :agent/stream-start :id event.id}}])
        (= event.phase :end)
        (let [successful (and event.ok state.terminal)
              next-event (if successful {:id event.id :type :agent/stream-end}
                             {:id event.id
                              :type :agent/stream-error
                              :message (or event.message
                                           (when (not state.terminal)
                                             "Codex stream ended without response.completed")
                                           event.body
                                           (.. "HTTP " (tostring event.status)))})]
          (stream-update event.id nil
                         (if state.failed []
                             [{:type :dispatch :event next-event}])))
        (or state.terminal state.failed)
        nil
        (do
          (var next-state state)
          (let [fx []]
            (var finished false)
            (each [_ record (ipairs (or event.records {})) &until finished]
              (let [handler (. (misa.catalog :codex-records) record.type)
                    result (and handler (handler next-state record event.id))]
                (when result
                  (set next-state (misa.patch next-state (or result.patch {})))
                  (each [_ effect (ipairs (or result.fx []))]
                    (table.insert fx effect))
                  (when result.finish
                    (set finished true)
                    (table.insert fx {:id event.id :type :operation/finish})))))
            (stream-update event.id next-state fx))))))

(fn build [context]
  "Describe openai codex policies for the supplied application settings."
  (let [declarations []
        providers (or (and (= (type context.config) :table)
                           context.config.providers) nil)]
    (var config (or (and (= (type providers) :table) providers.openai_codex)
                    nil))
    (set config (or (and (= (type config) :table) config) {}))
    (table.insert declarations
                  (let [definition {:description "ChatGPT subscription OAuth"
                                    :id :openai-codex
                                    :label "OpenAI Codex"
                                    :model_provider :openai-codex
                                    :discover_models (not config.models)
                                    :profile {:authorization_url "https://auth.openai.com/api/accounts/deviceauth/usercode"
                                              :id :default
                                              :token_url "https://auth.openai.com/oauth/token"}
                                    :strategy :device_oauth}]
                    {:catalog :auth-providers
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  {:catalog :events
                   :value {:event :usage/refresh
                           :handler (fn [db event]
                                      (when (or (not event.provider)
                                                (= event.provider :openai-codex))
                                        (let [provider (and db.providers
                                                            db.providers.openai-codex)]
                                          (if (and provider
                                                   provider.usage_request)
                                              {:patch {:providers {:openai-codex {:usage_again true}}}}
                                              (let [sequence (+ (or (and provider
                                                                         provider.usage_sequence)
                                                                    0)
                                                                1)
                                                    id (.. "codex-usage-"
                                                           sequence)]
                                                {:patch {:providers {:openai-codex {:usage_sequence sequence
                                                                                    :usage_request id}}}
                                                 :fx [{:type :http/request
                                                       :method :GET
                                                       : id
                                                       :url (or config.usage_url
                                                                "https://chatgpt.com/backend-api/wham/usage")
                                                       :credential {:id :openai-codex
                                                                    :header :authorization
                                                                    :prefix "Bearer "
                                                                    :metadata_field :account_id
                                                                    :metadata_header :chatgpt-account-id}
                                                       :response_format :json
                                                       :completion :provider/codex-usage
                                                       :timeouts config.timeouts}]})))))}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :provider/codex-usage
                           :handler (fn [db event cofx]
                                      (let [provider (and db.providers
                                                          db.providers.openai-codex)]
                                        (when (and provider
                                                   provider.usage_request
                                                   (= provider.usage_request
                                                      event.id))
                                          (let [windows (if event.ok
                                                            (usage-windows event.data)
                                                            [])]
                                            (each [_ window (ipairs windows)]
                                              (when (and (not window.reset_at_unix)
                                                         window.reset_after_seconds)
                                                (tset window :reset_at_unix
                                                      (+ (/ cofx.clock.wall_ms
                                                            1000)
                                                         window.reset_after_seconds))))
                                            (let [plan (when (and event.ok
                                                                  (= (type event.data)
                                                                     :table))
                                                         (quota-label event.data.plan_type))
                                                  count (when event.ok
                                                          (reset-count event.data))
                                                  details-id (when (and count
                                                                        (> count
                                                                           0))
                                                               (.. event.id
                                                                   "-resets"))
                                                  updated {:type :dispatch
                                                           :event {:type :usage/updated}}
                                                  fx [updated]]
                                              (when details-id
                                                (table.insert fx
                                                              {:type :http/request
                                                               :method :GET
                                                               :id details-id
                                                               :url (or config.reset_credits_url
                                                                        "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits")
                                                               :credential {:id :openai-codex
                                                                            :header :authorization
                                                                            :prefix "Bearer "
                                                                            :metadata_field :account_id
                                                                            :metadata_header :chatgpt-account-id}
                                                               :response_format :json
                                                               :completion :provider/codex-reset-credits
                                                               :timeouts config.timeouts}))
                                              (when provider.usage_again
                                                (table.insert fx
                                                              {:type :dispatch
                                                               :event {:type :usage/refresh
                                                                       :provider :openai-codex}}))
                                              {:patch {:providers {:openai-codex {:usage_request misa.delete
                                                                                  :usage_again misa.delete
                                                                                  :reset_count (misa.replace count)
                                                                                  :reset_credits_request (or details-id
                                                                                                             misa.delete)
                                                                                  :reset_credits misa.delete
                                                                                  :reset_refresh (if provider.usage_again
                                                                                                     provider.reset_refresh
                                                                                                     misa.delete)
                                                                                  :subscription_type (if event.ok
                                                                                                         (misa.replace plan)
                                                                                                         provider.subscription_type)
                                                                                  :usage (misa.replace {: windows
                                                                                                        :unavailable (= (length windows)
                                                                                                                        0)})}}}
                                               : fx})))))}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :provider/codex-reset-credits
                           :handler (fn [db event]
                                      (let [provider (and db.providers
                                                          db.providers.openai-codex)]
                                        (when (and provider
                                                   provider.reset_credits_request
                                                   (= event.id
                                                      provider.reset_credits_request))
                                          {:patch {:providers {:openai-codex {:reset_credits_request misa.delete
                                                                              :reset_credits (misa.replace (when event.ok
                                                                                                             (reset-credits event.data)))}}}
                                           :fx [{:type :dispatch
                                                 :event {:type :usage/updated}}]})))}})
    ;; The Codex backend client maps account/rateLimitResetCredit/consume
    ;; to this WHAM endpoint with redeem_request_id as its idempotency key.
    ;; Never send this request from setup, usage refresh, or a retry timer.
    (table.insert declarations
                  {:catalog :events
                   :value {:event :provider/codex-reset
                           :handler (fn [db _ cofx]
                                      (let [provider (and db.providers
                                                          db.providers.openai-codex)]
                                        (when (and provider
                                                   (not provider.reset_request)
                                                   (not provider.reset_refresh)
                                                   (or provider.reset_attempt
                                                       (> (or provider.reset_count
                                                              0)
                                                          0)))
                                          (let [sequence (+ (or provider.reset_sequence
                                                                0)
                                                            1)
                                                attempt (or provider.reset_attempt
                                                            (.. "misa-codex-reset-"
                                                                cofx.clock.wall_ms
                                                                "-"
                                                                cofx.clock.monotonic_ms
                                                                "-" sequence))
                                                id (.. "codex-reset-" sequence)]
                                            {:patch {:providers {:openai-codex {:reset_sequence sequence
                                                                                :reset_request id
                                                                                :reset_attempt attempt
                                                                                :reset_message "Resetting Codex quota…"}}}
                                             :fx [{:type :http/request
                                                   :method :POST
                                                   : id
                                                   :url "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume"
                                                   :credential {:id :openai-codex
                                                                :header :authorization
                                                                :prefix "Bearer "
                                                                :metadata_field :account_id
                                                                :metadata_header :chatgpt-account-id}
                                                   :headers [{:name :content-type
                                                              :value :application/json}]
                                                   :json {:redeem_request_id attempt}
                                                   :response_format :json
                                                   :completion :provider/codex-reset-complete
                                                   :timeouts config.timeouts}
                                                  {:type :dispatch
                                                   :event {:type :usage/updated}}]}))))}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :provider/codex-reset-complete
                           :handler (fn [db event]
                                      (let [provider (and db.providers
                                                          db.providers.openai-codex)]
                                        (when (and provider
                                                   provider.reset_request
                                                   (= event.id
                                                      provider.reset_request))
                                          (let [outcome (and event.ok
                                                             (= (type event.data)
                                                                :table)
                                                             event.data.outcome)
                                                messages {:reset "Codex quota reset used."
                                                          :already_redeemed "Codex quota reset already applied."
                                                          :nothing_to_reset "No Codex quota window is eligible for a reset."
                                                          :no_credit "No Codex quota resets are available."}
                                                message (and outcome
                                                             (. messages
                                                                outcome))]
                                            {:patch {:providers {:openai-codex {:reset_request misa.delete
                                                                                :reset_attempt (if message
                                                                                                   misa.delete
                                                                                                   provider.reset_attempt)
                                                                                :reset_refresh true
                                                                                :reset_message (or message
                                                                                                   "Could not confirm the Codex reset. Retry to check the same redemption.")}}}
                                             :fx [{:type :dispatch
                                                   :event {:type :usage/refresh
                                                           :provider :openai-codex}}
                                                  {:type :dispatch
                                                   :event {:type :usage/updated}}]}))))}})
    (let [serializer-id :openai.responses.codex]
      (table.insert declarations
                    {:catalog :serializers
                     :id serializer-id
                     :value {:accepts (fn [name]
                                        (= name :reasoning_effort))
                             :serialize (fn [name value]
                                          "Describe Codex request fields for a supported option."
                                          (when (= name :reasoning_effort)
                                            {:reasoning {:effort value
                                                         :summary :auto}}))}})
      (let [reasoning-api {:request_options {:reasoning_effort {:choices [:low
                                                                          :medium
                                                                          :high
                                                                          :xhigh]
                                                                :default :medium}}
                           :request_options_serializer serializer-id}]
        ;; ChatGPT's catalogue uses Codex slugs and reasoning metadata, rather
        ;; than the public API's /v1/models shape. Explicit catalogues stay static.
        (when (not config.models)
          (table.insert declarations
                        {:catalog :events
                         :value {:event :models/discover
                                 :handler (fn [_ event]
                                            (when (or (not event.provider)
                                                      (= event.provider
                                                         :openai-codex))
                                              {:fx [{:type :http/request
                                                     :id :models-openai-codex
                                                     :method :GET
                                                     :url (or config.models_url
                                                              (.. "https://chatgpt.com/backend-api/codex/models?client_version="
                                                                  (or config.client_version
                                                                      "0.153.4")))
                                                     :credential {:id :openai-codex
                                                                  :header :authorization
                                                                  :prefix "Bearer "
                                                                  :metadata_field :account_id
                                                                  :metadata_header :chatgpt-account-id}
                                                     :response_format :json
                                                     :completion :provider/codex-models
                                                     :timeouts config.timeouts}]}))}})
          (table.insert declarations
                        {:catalog :events
                         :value {:event :provider/codex-models
                                 :handler (fn [_ event]
                                            (let [fx []]
                                              (when (and event.ok
                                                         (= (type event.data)
                                                            :table)
                                                         (= (type event.data.models)
                                                            :table))
                                                (let [models []]
                                                  (each [_ item (ipairs event.data.models)]
                                                    (when (and (= (type item)
                                                                  :table)
                                                               (= (type item.slug)
                                                                  :string)
                                                               (not= item.slug
                                                                     "")
                                                               (= item.visibility
                                                                  :list))
                                                      (let [choices (icollect [_ level (ipairs (or item.supported_reasoning_levels
                                                                                                   []))]
                                                                      (when (and (= (type level)
                                                                                    :table)
                                                                                 (= (type level.effort)
                                                                                    :string))
                                                                        level.effort))
                                                            default (or item.default_reasoning_level
                                                                        (. choices
                                                                           1))
                                                            api (when (> (length choices)
                                                                         0)
                                                                  {:request_options {:reasoning_effort {: choices
                                                                                                        : default}}
                                                                   :request_options_serializer serializer-id})]
                                                        (table.insert models
                                                                      {:id (.. :openai-codex/
                                                                               item.slug)
                                                                       :model item.slug
                                                                       :label (or item.display_name
                                                                                  item.slug)
                                                                       :context_window item.context_window
                                                                       : api
                                                                       :created (when (= (type item.created)
                                                                                         :number)
                                                                                  item.created)
                                                                       :recommended (not= item.recommended
                                                                                          false)
                                                                       :recommendation_rank item.priority
                                                                       :popularity_rank (when (= (type item.popularity_rank)
                                                                                                 :number)
                                                                                          item.popularity_rank)}))))
                                                  ;; Invalid or entirely hidden responses should not erase the
                                                  ;; usable fallback catalogue (nor a prior successful discovery).
                                                  (when (> (length models) 0)
                                                    (table.sort models
                                                                (fn [a b]
                                                                  (< (or a.recommendation_rank
                                                                         math.huge)
                                                                     (or b.recommendation_rank
                                                                         math.huge))))
                                                    (table.insert fx
                                                                  {:type :dispatch
                                                                   :event {:type :models/replace-provider
                                                                           :provider :openai-codex
                                                                           :authoritative true
                                                                           : models}}))))
                                              (table.insert fx
                                                            {:type :dispatch
                                                             :event {:type :models/discovery-complete
                                                                     :provider :openai-codex}})
                                              {: fx}))}}))
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
          (let [api (if (and (= (type model.api) :table)
                             (= (type model.api.request_options) :table))
                        (misa.patch model.api
                                    {:request_options_serializer serializer-id})
                        model.api)]
            (table.insert declarations
                          (let [definition {: api
                                            :context_window model.context_window
                                            :id model.id
                                            :label (or model.label model.id)
                                            :model model.model
                                            :provider :openai-codex}]
                            {:catalog :models
                             :id (. definition :id)
                             :value definition}))))
        (table.insert declarations
                      {:catalog :effects
                       :id :provider.openai-codex
                       :value (fn [effect]
                                (let [body {:include [:reasoning.encrypted_content]
                                            :input (input effect.messages)
                                            :instructions (or effect.system_prompt
                                                              "You are a helpful coding assistant.")
                                            :model effect.model
                                            :parallel_tool_calls true
                                            :reasoning {:summary :auto}
                                            :store false
                                            :stream true
                                            :text {:verbosity :low}
                                            :tool_choice :auto}
                                      request-options (misa.request-options.serialize serializer-id
                                                                                      (or effect.request_options
                                                                                          {}))
                                      definitions (tools effect.tools)]
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
                                   :json (misa.patch body request-options)
                                   :method :POST
                                   :response_format :sse_json_stream
                                   :timeouts config.timeouts
                                   :type :http/request
                                   :url (or config.url
                                            "https://chatgpt.com/backend-api/codex/responses")}))})
        (each [id value (pairs records)]
          (table.insert declarations {:catalog :codex-records : id : value}))
        (table.insert declarations
                      {:catalog :validators
                       :id :codex-records
                       :value (fn [_ value]
                                (assert (= (type value) :function)
                                        "codex-records requires function definitions"))})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :provider/openai-codex-complete
                               :handler stream}})
        (definitions.build :provider.openai-codex declarations {})))))

{: build}
