(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:context_window 10
                                 :id :dynamic/old
                                 :label :Old
                                 :model :old
                                 :provider :dynamic}] {:catalog :models :id (. definition :id) :value definition}))
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:event {:authoritative true
                                                   :models [{:context_window 20
                                                             :id :dynamic/new
                                                             :label :New
                                                             :model :new}]
                                                   :provider :dynamic
                                                   :type :models/replace-provider}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :effects :id :provider.dynamic :value (fn [effect]
                                    (assert (= effect.model :new))
                                    {:event {:content [{:text "dynamic model"
                                                        :type :text}]
                                             :id effect.id
                                             :type :agent/result}
                                     :type :dispatch})})
          nil
          (definitions :tests.integration.fixtures.dynamic-models declarations {}))
