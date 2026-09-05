{:setup (fn []
          (assert (not (pcall (fn []
                                (misa.reg_cofx :clock (fn [] nil))
                                nil)))
                  "clock cofx name must be reserved")
          (var first nil)
          (misa.reg_event :app/start
                          (fn [_ _ cofx]
                            (assert (and (> cofx.clock.wall_ms 1000000000000)
                                         (>= cofx.clock.monotonic_ms 0))
                                    "native clocks are not trustworthy milliseconds")
                            (set first cofx.clock.monotonic_ms)
                            {:fx [{:event {:type :clock/next} :type :dispatch}]}))
          (misa.reg_event :clock/next
                          (fn [_ _ cofx]
                            (assert (>= cofx.clock.monotonic_ms first)
                                    "monotonic coeffect went backwards")
                            {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                     :text :clock}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

