{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/model
                         :value {:context_window 10
                                 :id :dynamic/old
                                 :label :Old
                                 :model :old
                                 :provider :dynamic}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:event {:authoritative true
                                                   :models [{:context_window 20
                                                             :id :dynamic/new
                                                             :label :New
                                                             :model :new}]
                                                   :provider :dynamic
                                                   :type :models/replace-provider}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.dynamic
                         :handler (fn [effect]
                                    (assert (= effect.model :new))
                                    {:event {:content [{:text "dynamic model"
                                                        :type :text}]
                                             :id effect.id
                                             :type :agent/result}
                                     :type :dispatch})})
          nil
          {:fx setup-fx})}
