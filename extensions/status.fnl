;; Agent facts and their status projections. Provider quota retrieval is owned
;; by providers; this feature consumes normalized usage, never transport records.

(fn metric [value]
  (if (not= (type value) :number) (tostring (or value "?"))
      (let [unit (accumulate [found nil _ item (ipairs [[1000000000 :G] [1000000 :M] [1000 :k]])
                              &until found]
                   (when (>= (math.abs value) (. item 1)) item))]
        (if unit
            (let [scaled (/ value (. unit 1))]
              (.. (: (string.format (if (>= (math.abs scaled) 10) "%.0f" "%.1f") scaled)
                     :gsub "%.0$" "") (. unit 2)))
            (tostring value)))))

(fn total [usage]
  (+ (or (and usage usage.input_tokens) 0) (or (and usage usage.output_tokens) 0)))

(fn usage-patch [event mode]
  {:mode mode
   :usage (when event.usage (misa.replace event.usage))
   :last_usage (when event.last_usage (misa.replace event.last_usage))
   :provider_usage (when event.provider_usage (misa.replace event.provider_usage))})

(fn usage-message [db]
  (local status (or db.status {}))
  (local usage (or status.usage {}))
  (local last (or status.last_usage {}))
  (local model (and misa.selected_model_projection
                     (misa.selected_model_projection db)))
  (local lines [(.. "Model: " (tostring (or (and model model.label) "none"))
                    (or (and model model.context_window
                             (.. "\nContext window: "
                                 (metric model.context_window))) "")
                    "\n\nSession usage"
                    "\nInput tokens: " (metric (or usage.input_tokens 0))
                    "\nOutput tokens: " (metric (or usage.output_tokens 0))
                    "\nLast request: " (metric (+ (or last.input_tokens 0)
                                                        (or last.output_tokens 0))))])
  (each [provider details (pairs (or db.providers {}))]
    (when (and (= (type details) :table) details.subscription_type)
      (table.insert lines (.. "\n" (tostring provider) " plan: "
                              (tostring details.subscription_type))))
    ;; Provider adapters may publish plan/quota facts without coupling the
    ;; dashboard to a provider-specific response shape.
    (when (and (= (type details) :table) details.usage)
      (local plan details.usage)
      (table.insert lines (.. "\n" (tostring provider) " plan usage: "
                              (tostring (or plan.summary plan.used plan))))))
  (each [provider plan (pairs (or status.provider_usage {}))]
    (table.insert lines (.. "\n" (tostring provider) " plan usage: "
                            (tostring (or plan.summary plan.used plan)))))
  (table.concat lines ""))


(local indicators
       [{:id :activity :icon "●" :label :activity
         :value (fn [db]
                  (local mode (or (and db.status db.status.mode) :ready))
                  (if (and (not= mode :ready) misa.animation_span)
                      {:spans [{:text mode} (misa.animation_span db :status)]}
                      mode))}
        {:id :session :icon :tok :label :tokens
         :value (fn [db] (metric (total (and db.status db.status.usage))))}
        {:id :context :icon "◫" :label :ctx
         :value (fn [db]
                  (local used (metric (total (and db.status db.status.last_usage))))
                  (local model (and misa.selected_model_projection (misa.selected_model_projection db)))
                  (if model (.. used "/" (metric model.context_window)) used))}])

(fn projection [db context]
  (if misa.indicators_projection (misa.indicators_projection db context)
      misa.render_component
      (. (misa.render_component db :status.metrics
                                {:metrics [{:prefix "●" :value (or (and db.status db.status.mode) :ready)}]}
                                context) :lines)
      []))

{:setup (fn []
          (local fx
                 [{:type :register/event :name :app/start
                   :handler (fn []
                              {:patch {:status (misa.replace {:last_usage {} :mode :ready :provider_usage {}
                                                              :usage {:input_tokens 0 :output_tokens 0}})}})}
                  {:type :register/event :name :agent/status
                   :handler (fn [_ event]
                              {:patch {:status (usage-patch event event.status)}})}
                  {:type :register/event :name :agent/usage
                   :handler (fn [_ event] {:patch {:status (usage-patch event)}})}
                  {:type :register/service :name :status_projection :value projection}
                  {:type :register/command
                   :value {:choice_purpose :command :description "Show token and coding-plan usage"
                           :event :usage/open :name :/usage}}
                  {:type :register/event :name :usage/open
                   :handler (fn [db]
                              {:fx [{:type :dispatch
                                     :event {:type :dialog/open :id :usage :title "Usage"
                                             :message (usage-message db)
                                             :actions [{:id :close :label "Close" :primary true}]
                                             :cancellable true :completion :usage/close :correlation :usage}}]})}
                  {:type :register/event :name :usage/close :handler (fn [] nil)}])
          (when (misa.has_setup_effect :register/indicator)
            (each [_ value (ipairs indicators)]
              (table.insert fx {:type :register/indicator : value})))
          {: fx})}
