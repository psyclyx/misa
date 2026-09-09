;; Provider normalizers feed semantic data rows. Effects are inspected only;
;; this fixture never dispatches transport effects or accesses accounts.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local misa _G.misa)
(each [_ name (ipairs [:misa.json :misa.ui.layout :misa.ui.values :misa.keybindings :misa.dialogs :misa.ui.components.buttons])]
  (app.define ((. (require name) :build) {:config {}})))
(set misa.time.local-datetime (fn [_] "localized time"))
(fn handlers [specs]
  (collect [_ spec (pairs specs.events)] spec.event spec.handler))
(local providers (collect [_ id (ipairs [:claude :openai-codex :kimi])]
                   (values id (handlers ((. (require (.. :misa.providers. id)) :build)
                                         {:config {}})))))
(local status-specs ((. (fennel.dofile :extensions/misa/ui/status/init.fnl) :build)))
(local usage-specs ((. (require :misa.usage) :build)))
(local status (handlers usage-specs))
(app.define {:subscriptions usage-specs.subscriptions})
(app.define {:subscriptions status-specs.subscriptions})
(app.install)
(local usage (handlers ((. (fennel.dofile :extensions/misa/usage/dialog.fnl) :build))))
(local dialogs (handlers ((. (fennel.dofile :extensions/misa/dialogs/init.fnl) :build))))
(local render-data (. ((. (fennel.dofile :extensions/misa/ui/components/data.fnl) :build)) :components :default.data :render))
(local render-dialog (. ((. (fennel.dofile :extensions/misa/dialogs/render.fnl) :build)) :components :default.dialog :render))
(local clock {:clock {:wall_ms 1788825600000 :monotonic_ms 0}})
(fn apply [registry db event]
  (local before (misa.json.encode db))
  (local result ((assert (. registry event.type)) db event clock))
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
(local (_ opened) (apply usage db {:type :usage/open}))
(assert (= (. opened 3 :event :type) :usage/refresh))
(assert (= (. opened 3 :event :provider) nil) "dashboard refresh excluded providers")
(set db (apply dialogs db (. opened 1 :event)))
(local original-dialog db.dialog)
(assert (= (length db.dialog.sections) 6))
(fn section [dialog id]
  (accumulate [found nil _ item (ipairs dialog.sections) &until found]
    (when (= item.id id) item)))
(fn row [dialog id label]
  (accumulate [found nil _ item (ipairs (. (assert (section dialog id)) :rows)) &until found]
    (when (= item.label label) item)))
(local extra (row db.dialog :claude "Extra usage"))
(assert (= (length extra.actions) 1))
(assert (= (. extra.fact.values 3 :amount) 12))
(assert (= (. extra.fact.values 5 :amount) 75))
(assert (= (. (section db.dialog :claude) :title) "claude · max"))
(assert (= (. (row db.dialog :kimi "Weekly limit") :meter :used) 10))
(assert (= (. (row db.dialog :kimi "Weekly limit") :detail :value) "2026-09-08T00:00:00Z"))
(assert (= (. (row db.dialog :openai-codex "Codex · 5h") :meter :used) 30))
(each [_ action (ipairs db.dialog.actions)] (assert (not action.primary)))
(assert (= db (apply dialogs db {:type :dialog/input :kind :enter})) "usage invented a default action")
(local (refreshing fx) (apply providers.openai-codex db {:type :usage/refresh :provider :openai-codex}))
(set db (apply providers.openai-codex refreshing {:type (. fx 1 :completion) :id (. fx 1 :id) :ok false}))
(assert (= (. (misa.sub (selected db :openai-codex) [:status/plan]) :type) :unavailable))
(local (_ update) (apply usage db {:type :usage/updated}))
(set db (apply dialogs db (. update 1 :event)))
(assert (= (. (row original-dialog :openai-codex "Codex · 5h") :meter :used) 30))
(assert (= (. (section db.dialog :openai-codex) :title) "openai-codex · pro"))
(assert (= (row db.dialog :openai-codex "Codex · 5h") nil) "stale quota remained visible")
(assert (= (. (row db.dialog :openai-codex "Quotas") :fact :value) "Temporarily unavailable"))
(local (_ absent) (apply usage (misa.patch db {:providers {:unknown {:usage {:unavailable true}}}})
                         {:type :usage/open}))
(assert (= (section (. absent 1 :event) :unknown) nil) "missing subscription invented a section")
(local data (render-data db.dialog {:columns 118}))
(local view (render-dialog (misa.patch db.dialog {:content data.lines :actions (misa.replace (misa.dialogs.buttons db.dialog))})
                           {:columns 120 :available_lines 100}))
(local text (table.concat (icollect [_ line (ipairs view.lines)]
                           (table.concat (icollect [_ span (ipairs line.spans)] span.text))) "\n"))
(assert (text:find "Extra usage" 1 true))
(assert (text:find "$12.00 / $75.00 this month" 1 true))
(assert (not (text:find "Plan usage" 1 true)))
(assert (not (text:find "next action" 1 true)))
(local (closed fx) (apply dialogs db {:type :dialog/input :kind :escape}))
(assert (= closed.dialog nil))
(assert (= (. fx 2 :type) :terminal/read) "closing lost input ownership")
(output "usage dashboard contracts passed\n")
