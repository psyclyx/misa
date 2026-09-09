(local animations (require :misa.ui.animations))
(local state (require :misa.ui.animations.state))
(local defaults (require :misa.ui.animations.default))

(fn settings [configuration] (or configuration.animations {}))
(fn options [config]
  (let [interval (or config.interval_ms 160)]
    (assert (or (= config.enabled nil) (= (type config.enabled) :boolean))
            "animations.enabled must be a boolean")
    (assert (and (= (type interval) :number) (= (% interval 1) 0)
                 (<= 10 interval 60000))
            "animations.interval_ms must be an integer from 10 through 60000")
    {:enabled (not= config.enabled false) :interval_ms interval}))

{:animations {:default defaults.pulse
              :static defaults.static
              :spinner defaults.spinner}
 :services {:animations.state animations.animations-state
            :animations.lookup animations.animations-lookup
            :animations.swap animations.animations-swap
            :animations.frame (fn [db role tick]
                                (animations.animations-frame (. (options (settings (misa.configuration)))
                                                                :enabled)
                                                             db role tick))
            :animations.span (fn [db role span-options]
                               (let [opts (options (settings (misa.configuration)))]
                                 (animations.animations-span opts.enabled
                                                             opts.interval_ms db
                                                             role span-options)))}
 :subscriptions {:animations/presentation {:id :animations/presentation
                                           :inputs (fn [query]
                                                     [[:db/path
                                                       :animations
                                                       :active]
                                                      [:db/path
                                                       :animations
                                                       :roles
                                                       (. query 2)]])
                                           :compute (fn [inputs query]
                                                      (let [opts (options (settings (misa.configuration)))]
                                                        (animations.animations-presentation-value opts.enabled
                                                                                                  opts.interval_ms
                                                                                                  inputs
                                                                                                  query)))}}
 :events {:animations/app/start {:event :app/start
                                 :handler (fn [db _ cofx]
                                            (let [config (settings cofx.config)]
                                              (options config)
                                              (animations.app-start config
                                                                    (or config.default
                                                                        :default)
                                                                    (or config.roles
                                                                        {})
                                                                    db)))}
          :animations/animations/loaded {:event :animations/loaded
                                         :handler (fn [db event cofx]
                                                    (animations.animations-loaded (options (settings cofx.config))
                                                                                  db
                                                                                  event))}
          :animations/animations/swap {:event :animations/swap
                                       :handler (fn [db event cofx]
                                                  (let [config (settings cofx.config)]
                                                    (animations.animations-swap-handler config
                                                                                        (options config)
                                                                                        db
                                                                                        event)))}
          :animations/animations/start {:event :animations/start
                                        :handler (fn [db event cofx]
                                                   (state.start (misa.catalog :animations)
                                                                (options (settings cofx.config))
                                                                db
                                                                (assert event.role
                                                                        "animation role is required")))}
          :animations/animations/stop {:event :animations/stop
                                       :handler (fn [db event cofx]
                                                  (state.stop (misa.catalog :animations)
                                                              (options (settings cofx.config))
                                                              db
                                                              (assert event.role
                                                                      "animation role is required")))}
          :animations/animations/tick {:event :animations/tick
                                       :handler (fn [db event cofx]
                                                  (state.tick (misa.catalog :animations)
                                                              (options (settings cofx.config))
                                                              db event))}}
 :validators {:animations animations.validate-animation}}
