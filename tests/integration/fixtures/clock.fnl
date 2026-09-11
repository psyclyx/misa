(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (var first nil)
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn [_ _ cofx]
                                    (assert (and (> cofx.clock.wall_ms
                                                    1000000000000)
                                                 (>= cofx.clock.monotonic_ms 0))
                                            "native clocks are not trustworthy milliseconds")
                                    (set first cofx.clock.monotonic_ms)
                                    {:fx [{:event {:type :clock/next}
                                           :type :dispatch}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :clock/next
                         :handler (fn [_ _ cofx]
                                    (assert (>= cofx.clock.monotonic_ms first)
                                            "monotonic coeffect went backwards")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :clock}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.clock declarations {}))
