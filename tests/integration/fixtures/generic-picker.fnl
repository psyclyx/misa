{:setup (fn []
          (misa.reg_command {:description "test generic picker"
                             :event :test/choose
                             :name :/choose})
          (misa.reg_command {:description "test picker panels"
                             :event :test/panels
                             :name :/panels})
          (misa.reg_event :test/choose
                          (fn [db]
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
                                   :type :dispatch}]}))
          (misa.reg_event :test/panels
                          (fn [db]
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
                                   :type :dispatch}]}))
          (misa.reg_event :test/chosen
                          (fn [_ event]
                            (local text
                                   (or (and event.cancelled
                                            (.. "cancelled " event.picker_token))
                                       event.value))
                            {:fx [{:lines [{:spans [{: text}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

