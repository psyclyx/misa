(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:event {:completion :hints/done
                                                   :id :hints
                                                   :items [{:label :One
                                                            :value :one}]
                                                   :title :Hints
                                                   :token "hints:1"
                                                   :type :picker/open}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :picker/open :handler (fn [db]
                                    (local layers
                                           (misa.ui.layers db
                                                             {:available_lines 18
                                                              :terminal {:columns 80
                                                                         :lines 20}}))
                                    (var found false)
                                    (each [_ line (ipairs (. layers 1 :lines))]
                                      (var text "")
                                      (each [_ item (ipairs line.spans)]
                                        (set text (.. text item.text)))
                                      (when (text:find "⌥z" 1 true)
                                        (set found true)))
                                    (assert found
                                            "configured picker hint disappeared")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :hints}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions.build :tests.integration.fixtures.picker-hints declarations {}))
