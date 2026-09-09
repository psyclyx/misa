(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:description "test generic picker"
                                 :event :test/choose
                                 :name :/choose}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        (let [definition {:description "test picker panels"
                                 :event :test/panels
                                 :name :/panels}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        {:catalog :events  :value {:event :test/choose :handler (fn [db]
                                    {
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
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/panels :handler (fn [db]
                                    {
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
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/chosen :handler (fn [_ event]
                                    (local text
                                           (or (and event.cancelled
                                                    (.. "cancelled "
                                                        event.picker_token))
                                               event.value))
                                    {:fx [{:lines [{:spans [{: text}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions.build :tests.integration.fixtures.generic-picker declarations {}))
