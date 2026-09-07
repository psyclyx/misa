;; Kimi Code subscription/API provider over Anthropic Messages.

(fn quota-number [value]
  (local number (tonumber value))
  (when (and number (= number number) (>= number 0) (< number math.huge)) number))

(fn quota-label [value] (when (= (type value) :string) value))

(fn usage-windows [payload]
  (local result [])
  (fn add [data label]
    (when (= (type data) :table)
      (local limit (quota-number data.limit))
      (local remaining (quota-number data.remaining))
      (local used (or (quota-number data.used)
                      (and limit remaining (math.max 0 (- limit remaining)))))
      (when (or used remaining limit)
        (table.insert result {:label (or (quota-label data.name) (quota-label data.title) label)
                              : limit : used
                              :remaining (or remaining (and limit used (math.max 0 (- limit used))))
                              :reset_at (quota-label (or data.reset_at data.resetAt data.reset_time data.resetTime))}))))
  (when (= (type payload) :table)
    (add payload.usage "Weekly limit")
    (each [index item (ipairs (if (= (type payload.limits) :table) payload.limits []))]
      (when (= (type item) :table)
        (add (or item.detail item) (or (quota-label item.name) (quota-label item.title)
                                      (quota-label item.scope) (.. "Limit " index))))))
  result)

{:setup (fn [context]
          (local setup-fx [])
          (assert (and misa.protocols misa.protocols.anthropic)
                  "provider.kimi requires protocol.anthropic first")
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (var config (or (and (= (type providers) :table) providers.kimi) nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local profiles
                 {:global {:api_base "https://api.kimi.ai/coding/v1"
                           :authorization_url "https://auth.kimi.ai/api/oauth/device_authorization"
                           :id :global
                           :token_url "https://auth.kimi.ai/api/oauth/token"}
                  :mainland {:api_base "https://api.kimi.com/coding/v1"
                             :authorization_url "https://auth.kimi.com/api/oauth/device_authorization"
                             :id :mainland
                             :token_url "https://auth.kimi.com/api/oauth/token"}})
          (local region (or config.region :global))
          (local profile
                 (assert (. profiles region)
                         "providers.kimi.region must be global or mainland"))
          (table.insert setup-fx
                        {:type :register/event :name :usage/refresh
                         :handler (fn [db]
                                    (local sequence (+ (or (and db.providers db.providers.kimi
                                                               db.providers.kimi.usage_sequence) 0) 1))
                                    (local id (.. "kimi-usage-" sequence))
                                    {:patch {:providers {:kimi {:usage_sequence sequence :usage_request id}}}
                                     :fx [{:type :http/request :method :GET : id
                                           :url (or config.usage_url (.. profile.api_base :/usages))
                                           :credential {:id :kimi-coding :header :authorization :prefix "Bearer "}
                                           :response_format :json :completion :provider/kimi-usage
                                           :timeouts config.timeouts}]})})
          (table.insert setup-fx
                        {:type :register/event :name :provider/kimi-usage
                         :handler (fn [db event]
                                    (local provider (and db.providers db.providers.kimi))
                                    (when (and provider provider.usage_request (= provider.usage_request event.id))
                                      (local windows (if event.ok (usage-windows event.data) []))
                                      {:patch {:providers {:kimi {:usage_request misa.delete
                                                                  :usage (misa.replace {: windows
                                                                                       :unavailable (= (length windows) 0)})}}}
                                       :fx [{:type :dispatch :event {:type :usage/updated}}]}))})
          (local models-url (or config.models_url
                                (or (and (and (not= config.discover_models
                                                    false)
                                              (= config.models nil))
                                         (.. profile.api_base :/models))
                                    nil)))
          (table.insert setup-fx
                        {:type :register/auth-provider
                         :value {:description (.. "Kimi coding plan OAuth ("
                                                  region ")")
                                 :discover_models (not= models-url nil)
                                 :id :kimi-coding
                                 :label "Kimi Coding"
                                 :model_provider :kimi
                                 : profile
                                 :strategy :device_oauth}})
          (each [_ declaration (ipairs (. (misa.protocols.anthropic {:auth_header :authorization
                                                                     :auth_prefix "Bearer "
                                                                     :catalogue_authoritative true
                                                                     :credential :kimi-coding
                                                                     :headers [{:name :user-agent
                                                                                :value :misa/0.1}]
                                                                     :id :kimi
                                                                     :max_tokens config.max_tokens
                                                                     :models (or config.models
                                                                                 {})
                                                                     :models_url models-url
                                                                     :timeouts config.timeouts
                                                                     :url (or config.url
                                                                              (.. profile.api_base
                                                                                  :/messages))})
                                          :fx))]
            (table.insert setup-fx declaration))
          {:fx setup-fx})}
