(local codex (require :misa.providers.openai-codex))
(local serializer :openai.responses.codex)
(local api {:request_options {:reasoning_effort {:choices [:low
                                                           :medium
                                                           :high
                                                           :xhigh]
                                                 :default :medium}}
            :request_options_serializer serializer})

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :openai_codex) {}))

(fn function-definition [_ value]
  (assert (= (type value) :function) "record projection must be a function"))

{:auth-providers {:openai-codex {:description "ChatGPT subscription OAuth"
                                 :id :openai-codex
                                 :label "OpenAI Codex"
                                 :model_provider :openai-codex
                                 :discover_models true
                                 :profile {:authorization_url "https://auth.openai.com/api/accounts/deviceauth/usercode"
                                           :id :default
                                           :token_url "https://auth.openai.com/oauth/token"}
                                 :strategy :device_oauth}}
 :serializers {serializer {:accepts (fn [name] (= name :reasoning_effort))
                           :serialize codex.serialize}}
 :effects {:provider.openai-codex (fn [effect]
                                    (codex.request (settings) serializer effect))}
 :events {:provider.openai-codex/usage-refresh {:event :usage/refresh
                                                :handler (fn [db event]
                                                           (codex.refresh-usage (settings)
                                                                                db
                                                                                event))
                                                :priority 9000}
          :provider.openai-codex/usage {:event :provider/codex-usage
                                        :handler (fn [db event cofx]
                                                   (codex.receive-usage (settings)
                                                                        db event
                                                                        cofx))
                                        :priority 9000}
          :provider.openai-codex/reset-credits {:event :provider/codex-reset-credits
                                                :handler codex.receive-reset-credits
                                                :priority 9000}
          :provider.openai-codex/reset {:event :provider/codex-reset
                                        :handler (fn [db event cofx]
                                                   (codex.reset-usage (settings)
                                                                      db event
                                                                      cofx))
                                        :priority 9000}
          :provider.openai-codex/reset-complete {:event :provider/codex-reset-complete
                                                 :handler codex.reset-complete
                                                 :priority 9000}
          :provider.openai-codex/discover {:event :models/discover
                                           :handler (fn [db event]
                                                      (when (. (misa.auth.for-model :openai-codex)
                                                               :discover_models)
                                                        (codex.discover-models (settings)
                                                                               db
                                                                               event)))
                                           :priority 9000}
          :provider.openai-codex/models {:event :provider/codex-models
                                         :handler (fn [db event]
                                                    (codex.receive-models serializer
                                                                          db
                                                                          event))
                                         :priority 9000}
          :provider.openai-codex/complete {:event :provider/openai-codex-complete
                                           :handler codex.stream
                                           :priority 9000}}
 :codex-records codex.records
 :validators {:codex-records function-definition}
 :models {:openai-codex/gpt-5.4 {:context_window 1000000
                                 :id :openai-codex/gpt-5.4
                                 :label "GPT-5.4 (ChatGPT)"
                                 :model :gpt-5.4
                                 : api
                                 :provider :openai-codex}
          :openai-codex/gpt-5.3-codex {:context_window 400000
                                       :id :openai-codex/gpt-5.3-codex
                                       :label "GPT-5.3 Codex"
                                       :model :gpt-5.3-codex
                                       : api
                                       :provider :openai-codex}}}
