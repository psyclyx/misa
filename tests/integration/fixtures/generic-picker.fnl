{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "test generic picker"
                                 :event :test/choose
                                 :name :/choose}})
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "test picker panels"
                                 :event :test/panels
                                 :name :/panels}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/choose
                         :handler (fn [db]
                                    {: db
                                     :fx [{:event {:completion :test/chosen
                                                   :id :test
                                                   :items [{:label :Alpha
                                                            :value :alpha}
                                                           {:label :Beta
                                                            :value :beta/path}]
                                                   :selected :alpha
                                                   :title :choice
                                                   :token "test:1"
                                                   :type :picker/open}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/panels
                         :handler (fn [db]
                                    {: db
                                     :fx [{:event {:completion :test/chosen
                                                   :id :panels
                                                   :panels [{:id :one
                                                             :items [{:value :alpha}]
                                                             :title :One}
                                                            {:id :two
                                                             :items [{:value :beta/path}]
                                                             :title :Two}
                                                            {:id :three
                                                             :items [{:value :gamma}]
                                                             :title :Three}]
                                                   :title :panels
                                                   :token "panels:1"
                                                   :type :picker/open}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/chosen
                         :handler (fn [_ event]
                                    (local text
                                           (or (and event.cancelled
                                                    (.. "cancelled "
                                                        event.picker_token))
                                               event.value))
                                    {:fx [{:lines [{:spans [{: text}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
