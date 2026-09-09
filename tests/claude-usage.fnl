(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local misa _G.misa)
(app.define ((fennel.dofile :extensions/json.fnl) {}))
(app.install)
(local feature (fennel.dofile :extensions/provider/claude.fnl))
;; Inspect declarations and invoke handlers only: never execute process effects.
(fn handlers-for [config]
  (local handlers {})
  (each [_ spec (pairs (. (feature {:config {:providers {:claude (or config {})}}}) :events))]
  (tset handlers spec.event spec.handler))
  handlers)
(local handlers (handlers-for nil))
(fn apply [db event]
  (local before (misa.json.encode db))
  (local input (fennel.view event))
  (local result ((assert (. handlers event.type) "Claude usage handler is missing") db event))
  (assert (= before (misa.json.encode db)) "Claude usage handler mutated old state")
  (assert (= input (fennel.view event)) "Claude usage handler mutated its completion")
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(fn option [argv name]
  (each [index value (ipairs argv)]
    (when (= value name) (lua "return argv[index + 1]")))
  nil)
(fn flag [argv name]
  (each [_ value (ipairs argv)] (when (= value name) (lua "return true")))
  false)
(local initial {:providers {:other {:untouched true}}})
(local (ignored ignored-fx) (apply initial {:type :usage/refresh :provider :other}))
(assert (= ignored initial))
(assert (= (length ignored-fx) 0))
(local (pending requests) (apply initial {:type :usage/refresh :provider :claude}))
(assert (= (length requests) 1))
(local request (. requests 1))
(assert (= request.type :provider/process))
(assert (= (. request.argv 1) :claude))
(assert (= request.completion :provider/claude-usage))
(assert (= request.stdout_format :json_lines))
(assert (and (= (type request.id) :string) (not= request.id "")))
(assert (= pending.providers.claude.usage_request request.id))
(assert (= pending.providers.claude.usage_sequence 1))
(assert (= pending.providers.other initial.providers.other))
(each [_ name (ipairs [:startup_ms :idle_ms :overall_ms])]
  (local timeout (. request.timeouts name))
  (assert (and (= (type timeout) :number) (> timeout 0) (< timeout math.huge))))
(assert (flag request.argv :--no-session-persistence))
(assert (= (option request.argv :--setting-sources) ""))
(assert (= (. (misa.json.decode (assert (option request.argv :--settings))) :disableAllHooks) true))
(assert (flag request.argv :--strict-mcp-config))
(local mcp (misa.json.decode (assert (option request.argv :--mcp-config))))
(assert (= (type mcp.mcpServers) :table))
(assert (= (next mcp.mcpServers) nil))
(assert (not (flag request.argv :--prompt)))
(assert (not (flag request.argv :--system-prompt)))
(assert (= request.stdin nil))
(assert (= request.stdin_json.type :control_request))
(assert (= request.stdin_json.request_id request.id))
(assert (= request.stdin_json.request.subtype :get_usage))
(assert (= request.stdin_json.message nil) "usage request contained a user prompt")
(local configured ((assert (. (handlers-for {:executable "/fixture/claude"}) :usage/refresh)) {} {}))
(assert (= (. configured.fx 1 :argv 1) "/fixture/claude"))
(local (queued no-requests) (apply pending {:type :usage/refresh}))
(assert queued.providers.claude.usage_again)
(assert (= (length no-requests) 0))
(assert (= queued (apply queued {:type :usage/refresh})))

(local reset "2026-09-08T00:00:00Z")
(local payload {:subscription_type :max :rate_limits_available true
                :rate_limits {:five_hour {:utilization 0 :resets_at reset}
                              :seven_day {:utilization 75}
                              :model_scoped [{:display_name :Fable :utilization 10}]}})
(fn records [id value subtype]
  [{:type :control_response
    :response {:subtype (or subtype :success) :request_id id :response value}}])
(fn completed [db id data ok]
  (apply db {:type :provider/claude-usage : id :ok (not= ok false) : data}))
(fn settled [db fx count]
  (assert (= db.providers.claude.usage_request nil))
  (assert (= db.providers.claude.usage_again nil))
  (assert (= (length fx) count))
  (assert (= (. fx 1 :type) :dispatch))
  (assert (= (. fx 1 :event :type) :usage/updated))
  (when (= count 2)
    (assert (= (. fx 2 :type) :dispatch))
    (assert (= (. fx 2 :event :type) :usage/refresh))
    (assert (= (. fx 2 :event :provider) :claude))))
(local (ready effects) (completed queued request.id (records request.id payload)))
(settled ready effects 2)
(local provider ready.providers.claude)
(assert (= provider.subscription_type :max))
(assert (= provider.usage.source :cli))
(assert (= provider.usage.unavailable false))
(assert (= (length provider.usage.windows) 3))
(local ids {})
(local by-used {})
(each [_ window (ipairs provider.usage.windows)]
  (assert (and (= (type window.id) :string) (not= window.id "") (not (. ids window.id))))
  (tset ids window.id true)
  (assert (and (= (type window.label) :string) (not= window.label "")))
  (assert (= window.unit :percent))
  (assert (= window.limit 100))
  (assert (= window.remaining (- 100 window.used)))
  (tset by-used window.used window))
(assert (= (. by-used 0 :remaining) 100) "zero utilization was treated as missing")
(assert (= (. by-used 0 :reset_at) reset))
(assert (= (. by-used 75 :remaining) 25))
(assert (: (. by-used 10 :label) :find :Fable 1 true))
(local (refreshing next-requests) (apply ready (. effects 2 :event)))
(local next-id (. next-requests 1 :id))
(assert (not= next-id request.id))
(assert (= refreshing.providers.claude.usage_sequence 2))
(local (stale stale-fx) (completed refreshing request.id (records request.id payload)))
(assert (= stale refreshing))
(assert (= (length stale-fx) 0))
(assert (= ready (completed ready request.id (records request.id payload))))
(assert (= ready (completed ready nil (records request.id payload))))

(fn unavailable [db id data ok again]
  (local (failed fx) (completed db id data ok))
  (settled failed fx (if again 2 1))
  (assert (= failed.providers.claude.usage.source :cli))
  (assert failed.providers.claude.usage.unavailable)
  (assert (= (length failed.providers.claude.usage.windows) 0))
  failed)
(unavailable pending request.id [] true)
(unavailable pending request.id nil true)
(unavailable pending request.id (records request.id payload :error) true)
(unavailable pending request.id (records :wrong-id payload) true)
(unavailable pending request.id (records request.id payload) false)
(unavailable queued request.id [] false true)
(unavailable refreshing next-id [] false)
;; Invalid quota values must never become zero-use or clamped quota windows.
(each [_ utilization (ipairs [misa.json-null math.huge (- math.huge) (/ 0 0) 101 -1])]
  (local (invalid fx) (completed pending request.id
                                (records request.id {:rate_limits_available true
                                                     :rate_limits {:five_hour {: utilization}}})))
  (settled invalid fx 1)
  (assert invalid.providers.claude.usage.unavailable)
  (each [_ window (ipairs invalid.providers.claude.usage.windows)]
    (assert (= window.used nil))
    (assert (= window.limit nil))
    (assert (= window.remaining nil))))
(unavailable pending request.id
             (records request.id {:rate_limits_available false}) true)
(local (full full-fx) (completed pending request.id
                                (records request.id {:rate_limits_available true
                                                     :rate_limits {:five_hour {:utilization 100}}})))
(settled full full-fx 1)
(assert (= (. full.providers.claude.usage.windows 1 :used) 100))
(assert (= (. full.providers.claude.usage.windows 1 :remaining) 0))
;; Ignore unrelated JSON-lines records while locating the matching response.
(local mixed [{:type :system :subtype :init}
              (. (records :other payload) 1)
              (. (records request.id payload) 1)])
(local mixed-ready (completed pending request.id mixed))
(assert (= (length mixed-ready.providers.claude.usage.windows) 3))
;; Scalar/null records and malformed control envelopes must be ignored safely.
(each [_ data (ipairs [false :garbage misa.json-null
                       [false 7 :garbage misa.json-null {}]
                       [{:type :control_response :response false}]
                       [{:type :control_response :response {:subtype :success :request_id request.id :response false}}]
                       [{:type :control_response :response {:subtype :success :request_id request.id :response misa.json-null}}]])]
  (unavailable pending request.id data true))
(local noise [false 7 :garbage misa.json-null {:type :control_response :response false}
              (. (records request.id payload) 1)])
(assert (= (length (. (completed pending request.id noise) :providers :claude :usage :windows)) 3))
(each [_ malformed (ipairs [{:rate_limits_available true :rate_limits false}
                            {:rate_limits_available true :rate_limits misa.json-null}
                            {:rate_limits_available true :rate_limits {:five_hour false
                                                                       :seven_day 3
                                                                       :model_scoped [false 9 {} {:display_name false}]}}])]
  (unavailable pending request.id (records request.id malformed) true))
;; Generic quota keys are deterministic; extra_usage is semantic data, never a window.
(local generic (completed pending request.id
                          (records request.id {:rate_limits_available true
                                               :rate_limits {:zeta {:utilization 20}
                                                             :alpha {:resets_at reset}
                                                             :extra_usage {:is_enabled false :currency :USD
                                                                           :decimal_places 2 :monthly_limit 12345
                                                                           :used_credits 0 :utilization 99}}})))
(local generic-usage generic.providers.claude.usage)
(assert (= (length generic-usage.windows) 2))
(assert (= (. generic-usage.windows 1 :id) :alpha))
(assert (= (. generic-usage.windows 1 :reset_at) reset))
(assert (= (. generic-usage.windows 1 :used) nil))
(assert (= (. generic-usage.windows 2 :id) :zeta))
(local extra generic-usage.extra_usage)
(assert (= extra.enabled false))
(assert (= extra.currency :USD))
(assert (= extra.limit 123.45))
(assert (= extra.used 0))
(assert (= extra.unlimited false))
(assert (= extra.manage_url "https://claude.ai/settings/usage"))
(each [_ decimals (ipairs [misa.json-null -1 1.5 10 math.huge])]
  (local minor (completed pending request.id
                          (records request.id {:rate_limits_available true
                                               :rate_limits {:extra_usage {:is_enabled true :currency :USD
                                                                           :decimal_places decimals
                                                                           :monthly_limit 12345 :used_credits 12}}})))
  (local amount minor.providers.claude.usage.extra_usage)
  (assert (= amount.limit 123.45))
  (assert (= amount.used 0.12)))
(each [_ decimals (ipairs [0 9])]
  (local scaled (completed pending request.id
                           (records request.id {:rate_limits_available true
                                                :rate_limits {:extra_usage {:is_enabled true :currency :USD
                                                                            :decimal_places decimals
                                                                            :monthly_limit 1000000000}}})))
  (assert (= scaled.providers.claude.usage.extra_usage.limit
             (/ 1000000000 (^ 10 decimals)))))
;; A targeted stream update replaces only its matching full-snapshot window.
(local full-snapshot (completed pending request.id
                               (records request.id {:subscription_type :max :rate_limits_available true
                                                    :rate_limits {:five_hour {:utilization 0}
                                                                  :seven_day {:utilization 75}
                                                                  :extra_usage {:is_enabled false :used_credits 12}}})))
(local full-usage full-snapshot.providers.claude.usage)
(local stream-window {:id :five_hour :label "Claude · 5h" :source :stream :unit :percent
                      :status :allowed :reset_at_unix 1800000000})
(local (merged merge-fx) (apply full-snapshot {:type :provider/claude-quota
                                             :usage {:source :stream :unavailable true :windows [stream-window]}}))
(local merged-usage merged.providers.claude.usage)
(assert (= merged.providers.claude.subscription_type :max))
(assert (= merged-usage.source :cli_stream))
(assert (= merged-usage.partial true))
(assert (= (length merged-usage.windows) 2))
(assert (= (. merged-usage.windows 1 :id) :five_hour))
(assert (= (. merged-usage.windows 1 :used) nil) "missing stream utilization retained stale quota")
(assert (= (. merged-usage.windows 1 :remaining) nil))
(assert (= (. merged-usage.windows 2 :id) :seven_day))
(assert (= (. merged-usage.windows 2 :used) 75))
(assert (= (. merged-usage.windows 2) (. full-usage.windows 2)) "stream merge replaced the untouched weekly record")
(assert (= merged-usage.extra_usage full-usage.extra_usage) "stream merge discarded extra usage settings")
(assert (= merged-usage.extra_usage.enabled false))
(assert (= (length merge-fx) 1))
(assert (= (. merge-fx 1 :event :type) :usage/updated))
(local unnamed (apply full-snapshot {:type :provider/claude-quota
                                     :usage {:source :stream :unavailable true
                                             :windows [{:label :Claude :unit :percent}]}}))
(assert (= (length unnamed.providers.claude.usage.windows) 1))
(assert (= unnamed.providers.claude.usage.source :stream))
(assert (= unnamed.providers.claude.usage.extra_usage nil) "unnamed stream window retained an uncorrelated full snapshot")
(output "Claude usage contracts passed\n")
