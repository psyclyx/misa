(local definitions (require :misa.definitions))

(fn [context]
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db event cofx]
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
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.context declarations {}))
