{:setup (fn [context]
          (local setup-fx [])
          (local ok (pcall (fn []
                             (misa._setup_effects {:fx [{:type :register/cofx
                                                         :name :terminal
                                                         :handler (fn [] nil)}]})
                             nil)))
          (assert (not ok) "terminal cofx name must be reserved")
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db event cofx]
                                    (assert (and (= cofx.config.value 42)
                                                 (= (. cofx.argv 1) :arg)))
                                    (assert (and (and (= cofx.terminal.interactive
                                                         false)
                                                      (= cofx.terminal.columns
                                                         37))
                                                 (= cofx.terminal.lines 11)))
                                    {:fx [{:lines [{:spans [{:style {:foreground :cyan}
                                                             :text :plain}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
