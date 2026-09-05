{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :picker/vendor/first
                                 :label :First
                                 :model :vendor/first
                                 :provider :picker}})
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :picker/vendor/second
                                 :label :Second
                                 :model :vendor/second
                                 :provider :picker}})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.picker
                         :handler (fn [effect]
                                    (assert (= effect.model :vendor/second))
                                    {:event {:content [{:text (.. "picked "
                                                                  effect.model)
                                                        :type :text}]
                                             :id effect.id
                                             :type :agent/result}
                                     :type :dispatch})})
          nil
          {:fx setup-fx})}
