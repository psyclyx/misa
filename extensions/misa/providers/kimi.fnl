(local protocol (require :misa.protocols.anthropic))

(local definitions (require :misa.definitions))

(fn quota-number [value]
  (let [number (tonumber value)]
    (when (and number (= number number) (>= number 0) (< number math.huge))
      number)))

(fn quota-label [value]
  (when (= (type value) :string) value))

(fn usage-windows [payload]
  "Normalize account usage into quota windows."
  (let [result []]
    (fn add [data label]
      (when (= (type data) :table)
        (let [limit (quota-number data.limit)
              remaining (quota-number data.remaining)
              used (or (quota-number data.used)
                       (and limit remaining (math.max 0 (- limit remaining))))]
          (when (or used remaining limit)
            (table.insert result
                          {:label (or (quota-label data.name)
                                      (quota-label data.title) label)
                           : limit
                           : used
                           :remaining (or remaining
                                          (and limit used
                                               (math.max 0 (- limit used))))
                           :reset_at (quota-label (or data.reset_at
                                                      data.resetAt
                                                      data.reset_time
                                                      data.resetTime))})))))

    (when (= (type payload) :table)
      (add payload.usage "Weekly limit")
      (each [index item (ipairs (if (= (type payload.limits) :table)
                                    payload.limits
                                    []))]
        (when (= (type item) :table)
          (add (or item.detail item)
               (or (quota-label item.name) (quota-label item.title)
                   (quota-label item.scope) (.. "Limit " index))))))
    result))

(fn refresh-usage [profile config db event]
  (when (or (not event.provider) (= event.provider :kimi))
    (let [provider (and db.providers db.providers.kimi)]
      (if (and provider provider.usage_request)
          {:patch {:providers {:kimi {:usage_again true}}}}
          (let [sequence (+ (or (and provider provider.usage_sequence) 0) 1)
                id (.. "kimi-usage-" sequence)]
            {:patch {:providers {:kimi {:usage_sequence sequence
                                        :usage_request id}}}
             :fx [{:type :http/request
                   :method :GET
                   : id
                   :url (or config.usage_url (.. profile.api_base :/usages))
                   :credential {:id :kimi-coding
                                :header :authorization
                                :prefix "Bearer "}
                   :response_format :json
                   :completion :provider/kimi-usage
                   :timeouts config.timeouts}]})))))

(fn receive-usage [db event]
  (let [provider (and db.providers db.providers.kimi)]
    (when (and provider provider.usage_request
               (= provider.usage_request event.id))
      (let [windows (if event.ok
                        (usage-windows event.data)
                        [])
            updated {:type :dispatch :event {:type :usage/updated}}]
        {:patch {:providers {:kimi {:usage_request misa.delete
                                    :usage_again misa.delete
                                    :usage (misa.replace {: windows
                                                          :unavailable (= (length windows)
                                                                          0)})}}}
         :fx (if provider.usage_again
                 [updated
                  {:type :dispatch
                   :event {:type :usage/refresh :provider :kimi}}]
                 [updated])}))))

(fn build [context]
  "Build the kimi provider catalogs from application settings."
  (let [declarations []
        providers (or (and (= (type context.config) :table)
                           context.config.providers) nil)
        raw-config (or (and (= (type providers) :table) providers.kimi) nil)
        config (or (and (= (type raw-config) :table) raw-config) {})
        profiles {:global {:api_base "https://api.kimi.ai/coding/v1"
                           :authorization_url "https://auth.kimi.ai/api/oauth/device_authorization"
                           :id :global
                           :token_url "https://auth.kimi.ai/api/oauth/token"}
                  :mainland {:api_base "https://api.kimi.com/coding/v1"
                             :authorization_url "https://auth.kimi.com/api/oauth/device_authorization"
                             :id :mainland
                             :token_url "https://auth.kimi.com/api/oauth/token"}}
        region (or config.region :global)
        profile (assert (. profiles region)
                        "providers.kimi.region must be global or mainland")]
    (table.insert declarations
                  {:catalog :events
                   :value {:event :usage/refresh
                           :handler (fn [db event]
                                      (refresh-usage profile config db event))}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :provider/kimi-usage :handler receive-usage}})
    (let [models-url (or config.models_url
                         (and (not= config.discover_models false)
                              (= config.models nil)
                              (.. profile.api_base :/models))
                         nil)]
      (table.insert declarations
                    (let [definition {:description (.. "Kimi coding plan OAuth ("
                                                       region ")")
                                      :discover_models (not= models-url nil)
                                      :id :kimi-coding
                                      :label "Kimi Coding"
                                      :model_provider :kimi
                                      : profile
                                      :strategy :device_oauth}]
                      {:catalog :auth-providers
                       :id (. definition :id)
                       :value definition}))
      (let [configured (protocol.configure {:auth_header :authorization
                                            :auth_prefix "Bearer "
                                            :catalogue_authoritative true
                                            :credential :kimi-coding
                                            :headers [{:name :user-agent
                                                       :value :misa/0.1}]
                                            :id :kimi
                                            :max_tokens config.max_tokens
                                            :models (or config.models {})
                                            :models_url models-url
                                            :timeouts config.timeouts
                                            :url (or config.url
                                                     (.. profile.api_base
                                                         :/messages))})]
        (definitions.build :provider.kimi declarations configured)))))

{:build build :usage-windows usage-windows}
