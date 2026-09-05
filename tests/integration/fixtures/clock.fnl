{:setup (fn []
          (local setup-fx [])
          (assert (not (pcall (fn []
                                (misa._setup_effects {:fx [{:type :register/cofx
                                                            :name :clock
                                                            :handler (fn [] nil)}]})
                                nil)))
                  "clock cofx name must be reserved")
          (var first nil)
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [_ _ cofx]
                                    (assert (and (> cofx.clock.wall_ms
                                                    1000000000000)
                                                 (>= cofx.clock.monotonic_ms 0))
                                            "native clocks are not trustworthy milliseconds")
                                    (set first cofx.clock.monotonic_ms)
                                    {:fx [{:event {:type :clock/next}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :clock/next
                         :handler (fn [_ _ cofx]
                                    (assert (>= cofx.clock.monotonic_ms first)
                                            "monotonic coeffect went backwards")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :clock}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
