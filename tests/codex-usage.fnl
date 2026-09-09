(local fennel (require :fennel))
(local output print)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local misa _G.misa)
(app.define ((fennel.dofile :extensions/json.fnl) {}))
(local status ((fennel.dofile :extensions/status.fnl)))
(app.define {:subscriptions status.subscriptions})
(app.install)
(local feature (fennel.dofile :extensions/provider/openai-codex.fnl))
(local handlers {})
(each [_ spec (pairs (. (feature {:config {}}) :events))]
  (tset handlers spec.event spec.handler))
(fn apply [db event]
  (local before (misa.json.encode db))
  (local result ((. handlers event.type) db event {:clock {:wall_ms 0 :monotonic_ms 0}}))
  (assert (= before (misa.json.encode db)) "quota handler mutated old state")
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(local initial {})
(assert (= initial (apply initial {:type :usage/refresh :provider :kimi})))
(local (pending fx) (apply initial {:type :usage/refresh :provider :openai-codex}))
(local request (. fx 1))
(assert (= request.url "https://chatgpt.com/backend-api/wham/usage"))
(assert (= request.method :GET))
(assert (= request.response_format :json))
(assert (= request.credential.id :openai-codex))
(assert (= request.credential.metadata_field :account_id))
(assert (= request.credential.metadata_header :chatgpt-account-id))
(assert (= request.headers nil) "quota request exposed auth header values")
(local (queued no-fx) (apply pending {:type :usage/refresh}))
(assert (= (length no-fx) 0))
(assert queued.providers.openai-codex.usage_again)
(assert (= queued (apply queued {:type :usage/refresh})))
;; Field structure observed from a successful native GET; values are synthetic.
(local payload {:account_id :private-account :email :private-email :user_id :private-user
                :plan_type :pro :code_review_rate_limit misa.json-null
                :rate_limit {:allowed true :limit_reached false
                             :primary_window {:used_percent 25 :limit_window_seconds 604800
                                              :reset_at 1800000000 :reset_after_seconds 3600}
                             :secondary_window misa.json-null}
                :additional_rate_limits [{:limit_name "Extra model" :metered_feature :extra
                                          :rate_limit {:primary_window {:used_percent 0 :limit_window_seconds 18000}
                                                       :secondary_window {:used_percent 100 :limit_window_seconds 604800}}}]})
(local (ready followup) (apply queued {:type :provider/codex-usage :id request.id :ok true :data payload}))
(local provider ready.providers.openai-codex)
(local windows provider.usage.windows)
(assert (= (length windows) 3))
(assert (= provider.subscription_type :pro))
(assert (= (. windows 1 :label) "Codex · 7d"))
(assert (= (. windows 1 :used) 25))
(assert (= (. windows 1 :remaining) 75))
(assert (= (. windows 1 :reset_at_unix) 1800000000))
(assert (= (. windows 1 :reset_after_seconds) 3600))
(assert (= (. windows 2 :label) "Extra model · 5h"))
(assert (= (. windows 2 :used) 0))
(assert (= (. windows 3 :remaining) 0))
(assert (= (. windows 1 :unit) :percent))
(assert (= provider.usage_request nil))
(assert (= provider.usage_again nil))
(local encoded-ready (misa.json.encode ready))
(assert (not (encoded-ready:find "private" 1 true)) "quota state retained account identifiers")
(assert (= (. followup 1 :event :type) :usage/updated))
(assert (= (. followup 2 :event :provider) :openai-codex))
(local (refreshing next-fx) (apply ready (. followup 2 :event)))
(local next-id (. next-fx 1 :id))
(assert (not= next-id request.id))
(assert (= refreshing (apply refreshing {:type :provider/codex-usage :id request.id :ok false})))
(local failed (apply refreshing {:type :provider/codex-usage :id next-id :ok false}))
(assert failed.providers.openai-codex.usage.unavailable)
(assert (= (length failed.providers.openai-codex.usage.windows) 0))
(assert (= failed.providers.openai-codex.subscription_type :pro))
(assert (= failed (apply failed {:type :provider/codex-usage :ok true :data payload})))
(local malformed (apply pending {:type :provider/codex-usage :id request.id :ok true
                                 :data {:plan_type {} :rate_limit {:primary_window {:used_percent -1}
                                                                 :secondary_window {:used_percent math.huge}}
                                        :additional_rate_limits [false {:limit_name {}
                                                                        :rate_limit {:primary_window {:used_percent "110"}}}]}}))
(assert (= (length malformed.providers.openai-codex.usage.windows) 1))
(assert (= (. malformed.providers.openai-codex.usage.windows 1 :remaining) 0))
(assert (= (. malformed.providers.openai-codex.usage.windows 1 :label) "Additional quota 2 · primary"))
;; The shared dashboard consumes normalized facts without provider-specific text.
(set misa.models.selected (fn [] {:provider :openai-codex}))
(var (open-usage plan-query) (values nil nil))
(each [_ spec (pairs (. ((fennel.dofile :extensions/usage.fnl)) :events))]
  (when (= spec.event :usage/open) (set open-usage spec.handler)))
(assert (= status.indicators.plan.value nil))
(set plan-query status.indicators.plan.query)
(fn plan-value [db]
  (misa.sub (misa.patch db {:models {:selected :codex
                                    :entries [{:id :codex :provider :openai-codex}]}}) plan-query))
(assert (= (. (plan-value ready) :value) 0))
(assert (= (. (plan-value ready) :type) :percent))
(assert (= (. (plan-value failed) :type) :unavailable))
(local dashboard (. (open-usage ready {} {:clock {:wall_ms 0 :monotonic_ms 0}}) :fx 1 :event))
(local facts {})
(each [_ section (ipairs dashboard.sections)]
  (each [_ row (ipairs (or section.rows []))] (tset facts row.label row)))
(assert (= (. facts "Codex · 7d" :meter :used) 25))
(assert (= (. facts "Codex · 7d" :detail :value) 1800000000))
(assert (= (. facts "Codex · 7d" :detail :type) :datetime))
(output "Codex usage contracts passed")
