{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/component
                         :id :test.status
                         :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text "independent status"}]}]})}})
          (table.insert setup-fx
                        {:type :register/component
                         :id :test.chrome
                         :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text "independent chrome"}]}]})}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:event {:implementation :test.status
                                                   :role :status.metrics
                                                   :type :components/swap}
                                           :type :dispatch}
                                          {:event {:type :test/status-swapped}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/status-swapped
                         :handler (fn [db]
                                    (assert (= (. (misa.render_component db
                                                                         :status.metrics
                                                                         {:metrics {}})
                                                  :lines 1 :spans 1 :text)
                                               "independent status"))
                                    (assert (= (. (misa.render_component db
                                                                         :root.header
                                                                         {})
                                                  :lines 1 :spans 2 :text)
                                               :misa)
                                            "status swap changed chrome")
                                    {:fx [{:event {:implementation :test.chrome
                                                   :role :root.header
                                                   :type :components/swap}
                                           :type :dispatch}
                                          {:event {:type :test/chrome-swapped}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/chrome-swapped
                         :handler (fn [db]
                                    (assert (= (. (misa.render_component db
                                                                         :status.metrics
                                                                         {:metrics {}})
                                                  :lines 1 :spans 1 :text)
                                               "independent status")
                                            "chrome swap changed status")
                                    {:fx [{:lines (. (misa.render_component db
                                                                            :root.header
                                                                            {})
                                                     :lines)
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
