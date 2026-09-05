;; Agent-owned facts normalized for generic indicator projections.

(fn metric [value]
  (if (not= (type value) :number) (tostring (or value "?"))
      (let [units [[1000000000 :G] [1000000 :M] [1000 :k]]]
        (each [_ unit (ipairs units)]
          (when (>= (math.abs value) (. unit 1))
            (local scaled (/ value (. unit 1)))
            (let [___antifnl_rtn_1___ (.. (: (or (and (>= scaled 10)
                                                      (string.format "%.0f"
                                                                     scaled))
                                                 (string.format "%.1f" scaled))
                                             :gsub "%.0$" "")
                                          (. unit 2))]
              (lua "return ___antifnl_rtn_1___"))))
        (tostring value))))

{:setup (fn []
          (misa.reg_event :app/start
                          (fn [db]
                            (set db.status
                                 {:last_usage {}
                                  :mode :ready
                                  :usage {:input_tokens 0 :output_tokens 0}})
                            {: db}))
          (misa.reg_event :agent/status
                          (fn [db event] (set db.status.mode event.status)
                            (when event.usage (set db.status.usage event.usage))
                            (when event.last_usage
                              (set db.status.last_usage event.last_usage))
                            {: db}))
          (misa.reg_event :agent/usage
                          (fn [db event]
                            (set db.status.usage
                                 (or event.usage db.status.usage))
                            (set db.status.last_usage
                                 (or event.last_usage db.status.last_usage))
                            {: db}))
          (if misa.reg_indicator
              (do
                (misa.reg_indicator {:icon "●"
                                     :id :activity
                                     :label :activity
                                     :value (fn [db]
                                              (var mode
                                                   (or (. (or db.status {})
                                                          :mode)
                                                       :ready))
                                              (when (and (not= mode :ready)
                                                         misa.animation_frame)
                                                (set mode
                                                     (.. mode
                                                         (misa.animation_frame db
                                                                               :status))))
                                              mode)})
                (misa.reg_indicator {:icon :tok
                                     :id :session
                                     :label :tokens
                                     :value (fn [db]
                                              (local usage
                                                     (or (. (or db.status {})
                                                            :usage)
                                                         {}))
                                              (metric (+ (or usage.input_tokens
                                                             0)
                                                         (or usage.output_tokens
                                                             0))))})
                (misa.reg_indicator {:icon "◫"
                                     :id :context
                                     :label :ctx
                                     :value (fn [db]
                                              (local last
                                                     (or (. (or db.status {})
                                                            :last_usage)
                                                         {}))
                                              (local used
                                                     (+ (or last.input_tokens 0)
                                                        (or last.output_tokens
                                                            0)))
                                              (local model
                                                     (or (and misa.selected_model_projection
                                                              (misa.selected_model_projection db))
                                                         nil))
                                              (or (and model
                                                       (.. (metric used) "/"
                                                           (metric model.context_window)))
                                                  (metric used)))})

                (fn misa.status_projection [db context]
                  (misa.indicators_projection db context)))
              (do
                ;; Small compatibility projection for profiles that intentionally omit the
                ;; indicators service.

                (fn misa.status_projection [db]
                  (if (not misa.render_component) {}
                      (do
                        (local state (or db.status {}))
                        (. (misa.render_component db :status.metrics
                                                  {:metrics [{:prefix "●"
                                                              :value (or state.mode
                                                                         :ready)}]})
                           :lines))))))
          nil)}

