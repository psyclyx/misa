{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:event {:completion :hints/done
                                                   :id :hints
                                                   :items [{:label :One
                                                            :value :one}]
                                                   :title :Hints
                                                   :token "hints:1"
                                                   :type :picker/open}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :picker/open
                         :handler (fn [db]
                                    (local layers
                                           (misa.view_layers db
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
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
