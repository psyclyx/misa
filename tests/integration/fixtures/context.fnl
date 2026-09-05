{:setup (fn [context]
          (local ok (pcall (fn []
                             (misa.reg_cofx :terminal (fn [] nil))
                             nil)))
          (assert (not ok) "terminal cofx name must be reserved")
          (misa.reg_event :app/start
                          (fn [db event cofx]
                            (assert (and (= cofx.config.value 42)
                                         (= (. cofx.argv 1) :arg)))
                            (assert (and (and (= cofx.terminal.interactive
                                                 false)
                                              (= cofx.terminal.columns 37))
                                         (= cofx.terminal.lines 11)))
                            {:fx [{:lines [{:spans [{:style {:foreground :cyan}
                                                     :text :plain}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

