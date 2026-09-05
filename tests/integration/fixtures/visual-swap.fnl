{:setup (fn []
          (misa.reg_component :test.status
                              {:render (fn []
                                         {:lines [{:spans [{:style :plain
                                                            :text "independent status"}]}]})})
          (misa.reg_component :test.chrome
                              {:render (fn []
                                         {:lines [{:spans [{:style :plain
                                                            :text "independent chrome"}]}]})})
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:event {:implementation :test.status
                                           :role :status.metrics
                                           :type :components/swap}
                                   :type :dispatch}
                                  {:event {:type :test/status-swapped}
                                   :type :dispatch}]}))
          (misa.reg_event :test/status-swapped
                          (fn [db]
                            (assert (= (. (misa.render_component db
                                                                 :status.metrics
                                                                 {:metrics {}})
                                          :lines 1 :spans 1 :text)
                                       "independent status"))
                            (assert (= (. (misa.render_component db
                                                                 :root.header {})
                                          :lines 1 :spans 2 :text)
                                       :misa)
                                    "status swap changed chrome")
                            {:fx [{:event {:implementation :test.chrome
                                           :role :root.header
                                           :type :components/swap}
                                   :type :dispatch}
                                  {:event {:type :test/chrome-swapped}
                                   :type :dispatch}]}))
          (misa.reg_event :test/chrome-swapped
                          (fn [db]
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
                                  {:type :app/quit}]}))
          nil)}

