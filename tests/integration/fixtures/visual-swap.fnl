(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :components :id :test.status :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text "independent status"}]}]})}})
          (table.insert declarations
                        {:catalog :components :id :test.chrome :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text "independent chrome"}]}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:event {:implementation :test.status
                                                   :role :status.indicators
                                                   :type :components/swap}
                                           :type :dispatch}
                                          {:event {:type :test/status-swapped}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/status-swapped :handler (fn [db]
                                    (assert (= (. (misa.components.render db
                                                                         :status.indicators
                                                                         {:indicators {}})
                                                  :lines 1 :spans 1 :text)
                                               "independent status"))
                                    (assert (= (. (misa.components.render db
                                                                         :root.header
                                                                         {})
                                                  :lines 1 :spans 1 :text)
                                               :misa)
                                            "status swap changed chrome")
                                    {:fx [{:event {:implementation :test.chrome
                                                   :role :root.header
                                                   :type :components/swap}
                                           :type :dispatch}
                                          {:event {:type :test/chrome-swapped}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/chrome-swapped :handler (fn [db]
                                    (assert (= (. (misa.components.render db
                                                                         :status.indicators
                                                                         {:indicators {}})
                                                  :lines 1 :spans 1 :text)
                                               "independent status")
                                            "chrome swap changed status")
                                    {:fx [{:lines (. (misa.components.render db
                                                                            :root.header
                                                                            {})
                                                     :lines)
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.visual-swap declarations {}))
