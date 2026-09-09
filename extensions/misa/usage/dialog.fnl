(local definitions (require :misa.definitions))

;; Usage projects normalized provider facts into generic data rows and actions.
;; Transport and account mutations remain provider-owned.
(fn text [value] {:type :text :value value})
(fn tokens [value] {:type :tokens :value (or value 0)})
(fn sequence [parts] {:type :sequence :values parts})
(fn datetime [instant now prefix fallback]
  {:type :datetime :value instant :relative_to now : prefix : fallback})

(fn value-text [fact]
  (table.concat (icollect [_ part (ipairs (misa.values.render fact))]
                  part.text)))

(fn window-row [window now]
  (let [used (or window.used
                 (and window.limit window.remaining
                      (- window.limit window.remaining)))
        instant (or window.reset_at_unix window.reset_at)]
    {:label (or window.label "Quota")
     :meter {: used :limit window.limit}
     :fact (if (and used (= window.unit :percent))
               (sequence [{:type :percent
                           :value (math.max 0 (math.min 100 used))}
                          (text " used")])
               (sequence [{:type :ratio : used :limit window.limit}
                          (text " used")]))
     :detail (when instant (datetime instant now "Resets "))}))

(fn extra-row [provider extra actions]
  (let [facts [(text (if extra.enabled "On" "Off"))]]
    (when extra.used
      (table.insert facts (text " · "))
      (table.insert facts {:type :money
                           :amount extra.used
                           :currency extra.currency})
      (when extra.limit
        (table.insert facts (text " / "))
        (table.insert facts
                      {:type :money
                       :amount extra.limit
                       :currency extra.currency}))
      (table.insert facts (text (if extra.unlimited " used · no monthly limit"
                                    " this month"))))
    (let [ids []]
      (when extra.manage_url
        (let [id (.. provider "/extra-manage")]
          (table.insert ids id)
          (table.insert actions
                        {: id
                         :label "Manage"
                         :inline true
                         :persistent true
                         :event {:type :link/open :url extra.manage_url}
                         :binding (when (= provider :claude)
                                    {:context :usage :action :extra-manage})})))
      {:label "Extra usage" :fact (sequence facts) :actions ids})))

(fn reset-rows [provider now rows actions]
  (when (or provider.reset_count provider.reset_attempt)
    (let [count (or provider.reset_count 0)
          ids []]
      (when (or (> count 0) provider.reset_attempt)
        (table.insert ids :codex-reset)
        (let [expiries []]
          (each [_ credit (ipairs (or provider.reset_credits []))]
            (when (= credit.status :available)
              (table.insert expiries
                            (if credit.expires_never "No expiry"
                                (value-text (datetime (or credit.expires_at_unix
                                                          credit.expires_at)
                                                      now "Expires "
                                                      "Expiry unavailable"))))))
          (table.insert actions
                        {:id :codex-reset
                         :label (if provider.reset_attempt "Retry reset"
                                    "Use reset")
                         :inline true
                         :persistent true
                         :event {:type :provider/codex-reset}
                         :disabled (or provider.reset_request
                                       provider.reset_refresh)
                         :binding {:context :usage :action :codex-reset}
                         :confirm {:title "Use a Codex quota reset?"
                                   :label "Use reset"
                                   :message (.. (if provider.reset_attempt
                                                    "Check the previous redemption again; a retry uses the same request key."
                                                    "Consume one earned reset to reset eligible Codex usage limits.")
                                                (if (> (length expiries) 0)
                                                    (.. "\n"
                                                        (table.concat expiries
                                                                      "\n"))
                                                    "\nReset expiry details are unavailable."))}})))
      (table.insert rows {:label "Quota resets"
                          :fact (text (.. count " available"))
                          :actions ids})
      (each [index credit (ipairs (or provider.reset_credits []))]
        (when (= credit.status :available)
          (table.insert rows
                        {:label (.. "Reset " index)
                         :fact (if credit.expires_never (text "No expiry")
                                   (datetime (or credit.expires_at_unix
                                                 credit.expires_at)
                                             now "Expires " "Expiry unavailable"))})))
      (when (and (> count 0) (not provider.reset_credits))
        (table.insert rows
                      {:label "Reset expiry"
                       :fact (text (if provider.reset_credits_request
                                       "Loading…"
                                       "Unavailable"))}))))
  (when provider.reset_message
    (table.insert rows {:label "" :fact (text provider.reset_message)})))

(fn model [db now]
  (let [usage (or (misa.sub db [:usage/session]) {})
        last (or (misa.sub db [:usage/last-request]) {})
        selected (and misa.models misa.models.selected
                      (misa.models.selected db))
        sections [{:id :model
                   :rows [{:label "Model"
                           :fact (text (or (and selected selected.label) "None"))}
                          {:label "Context window"
                           :fact (if (and selected selected.context_window)
                                     (tokens selected.context_window)
                                     (text "Unknown"))}]}
                  {:id :session
                   :title "Session usage"
                   :heading true
                   :rows [{:label "Input tokens"
                           :fact (tokens usage.input_tokens)}
                          {:label "Output tokens"
                           :fact (tokens usage.output_tokens)}
                          {:label "Last request"
                           :fact (tokens (+ (or last.input_tokens 0)
                                            (or last.output_tokens 0)))}]}]
        actions []
        providers {}]
    (each [id details (pairs (or db.providers {}))]
      (when (and (= (type details) :table)
                 (or details.subscription_type details.usage))
        (tset providers id {:plan details.subscription_type
                            :usage details.usage})))
    (each [id plan (pairs (or (misa.sub db [:usage/providers]) {}))]
      (tset providers id {:plan (and (. providers id) (. providers id :plan))
                          :usage plan}))
    (let [ids (icollect [id (pairs providers)] id)]
      (table.sort ids)
      (var grouped false)
      (each [_ id (ipairs ids)]
        (let [provider (. providers id)
              quota provider.usage]
          (when (and (= (type quota) :table)
                     (or provider.plan
                         (and (not quota.unavailable)
                              (> (length (or quota.windows [])) 0))))
            (when (not grouped)
              (table.insert sections
                            {:id :subscriptions
                             :title "Subscription plans"
                             :heading true})
              (set grouped true))
            (let [rows (icollect [_ window (ipairs (or quota.windows []))]
                         (window-row window now))]
              (when quota.unavailable
                (table.insert rows
                              {:label "Quotas"
                               :fact (text "Temporarily unavailable")}))
              (when quota.extra_usage
                (table.insert rows (extra-row id quota.extra_usage actions)))
              (when (= id :openai-codex)
                (reset-rows (or (and db.providers (. db.providers id)) {}) now
                            rows actions))
              (table.insert sections
                            {:id id
                             :title (.. id
                                        (if provider.plan
                                            (.. " · " provider.plan)
                                            ""))
                             : rows})))))
      {: sections : actions})))

(fn update [db _ cofx]
  (when (and db.dialog (= db.dialog.id :usage))
    (let [display (model db (/ cofx.clock.wall_ms 1000))]
      {:fx [{:type :dispatch
             :event {:type :dialog/update
                     :id :usage
                     :correlation :usage
                     :sections display.sections
                     :actions display.actions}}]})))

(fn build []
  "Declare the usage dashboard and its interactions."
  (definitions.build :usage
    [(let [definition {:context :usage :action :codex-reset :default ["r"]}]
       {:catalog :keybindings
        :id (.. (. definition :context) "/" (. definition :action))
        :value definition})
     (let [definition {:context :usage :action :extra-manage :default ["e"]}]
       {:catalog :keybindings
        :id (.. (. definition :context) "/" (. definition :action))
        :value definition})
     (let [definition {:id :usage.open
                       :label "Show usage"
                       :event {:type :usage/open}
                       :available (fn [db] (not db.dialog))}]
       {:catalog :actions :id (. definition :id) :value definition})
     (let [definition {:choice_purpose :command
                       :description "Show token and subscription usage"
                       :event :usage/open
                       :name :/usage}]
       {:catalog :commands :id (. definition :name) :value definition})
     {:catalog :events
      :value {:event :usage/open
              :handler (fn [db _ cofx]
                         (let [display (model db (/ cofx.clock.wall_ms 1000))]
                           {:fx [{:type :dispatch
                                  :event {:type :dialog/open
                                          :id :usage
                                          :title "Usage"
                                          :sections display.sections
                                          :actions display.actions
                                          :cancellable true
                                          :completion :usage/action
                                          :correlation :usage}}
                                 {:type :timer/start
                                  :id :usage/countdown
                                  :interval_ms 60000
                                  :completion :usage/tick}
                                 {:type :dispatch
                                  :event {:type :usage/refresh}}]}))}}
     {:catalog :events :value {:event :usage/updated :handler update}}
     {:catalog :events
      :value {:event :usage/tick
              :handler (fn [db event cofx]
                         (or (update db event cofx)
                             {:fx [{:type :timer/stop :id :usage/countdown}]}))}}
     {:catalog :events
      :value {:event :usage/action
              :handler (fn [db event]
                         (if event.cancelled
                             {:fx [{:type :timer/stop :id :usage/countdown}]}
                             (when (and db.dialog (= db.dialog.id :usage))
                               (let [action (accumulate [found nil _ action (ipairs db.dialog.actions)]
                                              (if (= action.id event.action)
                                                  action found))]
                                 (when (and action action.event
                                            (not action.disabled))
                                   {:fx [{:type :dispatch
                                          :event (misa.snapshot action.event)}]})))))}}]
    {}))

{: build}
