;; Real provider normalizers feed one dashboard and selected-provider fact query.
;; Native request effects are inspected only: no account or network access.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(each [_ name (ipairs [:json :layout])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) {}))
(set misa.protocols {:anthropic (fn [] {:fx []})})
(fn handlers [specs]
  (collect [_ spec (ipairs specs.fx)]
    (when (= spec.type :register/event) (values spec.name spec.handler))))
(local providers (collect [_ id (ipairs [:claude :openai-codex :kimi])]
                   (values id (handlers ((. (fennel.dofile (.. :extensions/provider/ id :.fnl)) :setup)
                                         {:config {}})))))
(local status-specs ((. (fennel.dofile :extensions/status.fnl) :setup)))
(local status (handlers status-specs))
(each [_ spec (ipairs status-specs.fx)]
  (when (= spec.type :register/sub) (misa._setup_effects {:fx [spec]})))
(local dialogs (handlers ((. (fennel.dofile :extensions/dialogs.fnl) :setup))))
(local render (. ((. (fennel.dofile :extensions/component/dialog.fnl) :setup)) :fx 1 :value :render))
(fn apply [registry db event]
  (local before (misa.json.encode db))
  (local result ((assert (. registry event.type)) db event))
  (assert (= before (misa.json.encode db)) "usage flow mutated a snapshot")
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(var db (apply status {} {:type :app/start}))
(local requests {})
(each [id registry (pairs providers)]
  (local (pending fx) (apply registry db {:type :usage/refresh}))
  (set db pending)
  (assert (= (length fx) 1))
  (tset requests id (. fx 1)))
(assert (= requests.claude.stdin_json.request.subtype :get_usage))
(assert (= requests.openai-codex.credential.id :openai-codex))
(assert (= requests.kimi.credential.id :kimi-coding))
(local payloads
       {:claude [{:type :control_response
                  :response {:subtype :success :request_id requests.claude.id
                             :response {:subscription_type :max :rate_limits_available true
                                        :rate_limits {:five_hour {:utilization 100}
                                                      :extra_usage {:is_enabled true :currency :USD
                                                                    :decimal_places 2 :monthly_limit 7500
                                                                    :used_credits 1200}}}}}]
        :openai-codex {:plan_type :pro :rate_limit {:primary_window {:used_percent 30 :limit_window_seconds 18000}}}
        :kimi {:usage {:limit 50 :remaining 40 :resetAt "2026-09-08T00:00:00Z"}}})
(each [id registry (pairs providers)]
  (local request (. requests id))
  (local (settled fx) (apply registry db {:type request.completion :id request.id :ok true :data (. payloads id)}))
  (set db settled)
  (assert (= (. fx 1 :event :type) :usage/updated))
  (assert (= (length (. db.providers id :usage :windows)) 1)))
(fn selected [state id]
  (misa.patch state {:models (misa.replace {:selected id :entries [{:id id :provider id}]})}))
(each [id expected (pairs {:claude 0 :openai-codex 70 :kimi 80})]
  (local fact (misa.sub (selected db id) [:status/plan]))
  (assert (= fact.type :percent))
  (assert (= fact.value expected)))
(assert (= (misa.sub (selected db :absent) [:status/plan]) nil))
(local (_ opened) (apply status db {:type :usage/open}))
(assert (= (. opened 2 :event :type) :usage/refresh))
(assert (= (. opened 2 :event :provider) nil) "dashboard refresh excluded providers")
(set db (apply dialogs db (. opened 1 :event)))
(local original-dialog db.dialog)
(assert (= (length db.dialog.sections) 5))
(fn fields [dialog id]
  (local section (assert (accumulate [found nil _ item (ipairs dialog.sections) &until found]
                           (when (= item.id id) item))
                         (.. "missing section " id)))
  (collect [_ field (ipairs section.fields)] (values field.label field.value)))
(local claude (fields db.dialog :claude))
(assert (= claude.Plan :max))
(assert (= (. claude "Extra usage used (USD)") 12))
(assert (= (. claude "Extra usage monthly limit (USD)") 75))
(assert (= (. (fields db.dialog :kimi) "Weekly limit remaining") 40))
(assert (= (. (fields db.dialog :kimi) "Weekly limit resets at") "2026-09-08T00:00:00Z"))
(assert (= (. (fields db.dialog :openai-codex) "Codex · 5h used") 30))
(local (refreshing fx) (apply providers.openai-codex db {:type :usage/refresh :provider :openai-codex}))
(set db (apply providers.openai-codex refreshing {:type (. fx 1 :completion) :id (. fx 1 :id) :ok false}))
(assert (= (. (misa.sub (selected db :openai-codex) [:status/plan]) :type) :unavailable))
(assert (= (. (misa.sub (selected db :kimi) [:status/plan]) :value) 80))
(local (_ update) (apply status db {:type :usage/updated}))
(set db (apply dialogs db (. update 1 :event)))
(assert (= (. (fields original-dialog :openai-codex) "Codex · 5h used") 30))
(assert (= (. (fields db.dialog :openai-codex) "Codex · 5h used") nil))
(assert (= (. (fields db.dialog :openai-codex) "Plan usage") "Unavailable"))
(assert (= (. (fields db.dialog :kimi) "Weekly limit remaining") 40))
(local view (render db.dialog {:columns 120 :available_lines 100}))
(local text (table.concat (icollect [_ line (ipairs view.lines)]
                           (table.concat (icollect [_ span (ipairs line.spans)] span.text))) "\n"))
(assert (text:find "Extra usage used (USD)" 1 true))
(assert (text:find "Unavailable" 1 true))
(output "usage dashboard contracts passed\n")
