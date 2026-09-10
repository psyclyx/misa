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
  "Describe a Kimi account usage request."
  (when (or (not event.provider) (= event.provider :kimi))
    (let [provider (and db.providers db.providers.kimi)]
      (if (and provider provider.usage_request)
          {:patch {:providers {:kimi {:usage_again true}}}}
          (let [sequence (+ (or (and provider provider.usage_sequence) 0) 1)
                id (.. :kimi-usage- sequence)]
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
  "Apply completed Kimi account usage."
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

(local profiles
       {:global {:api_base "https://api.kimi.ai/coding/v1"
                 :authorization_url "https://auth.kimi.ai/api/oauth/device_authorization"
                 :id :global
                 :token_url "https://auth.kimi.ai/api/oauth/token"}
        :mainland {:api_base "https://api.kimi.com/coding/v1"
                   :authorization_url "https://auth.kimi.com/api/oauth/device_authorization"
                   :id :mainland
                   :token_url "https://auth.kimi.com/api/oauth/token"}})

(fn settings [config supplied-profile]
  "Describe provider transport settings."
  (let [profile (or supplied-profile profiles.global)
        models-url (or config.models_url (.. profile.api_base :/models))]
    {:auth_header :authorization
     :auth_prefix "Bearer "
     :catalogue_authoritative true
     :credential :kimi-coding
     :headers [{:name :user-agent :value :misa/0.1}]
     :id :kimi
     :max_tokens config.max_tokens
     :models_url models-url
     :timeouts config.timeouts
     :url (or config.url (.. profile.api_base :/messages))}))

{: settings : profiles : refresh-usage : receive-usage : usage-windows}
