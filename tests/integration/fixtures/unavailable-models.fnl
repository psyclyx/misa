{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/auth-provider
                         :value {:id :openai
                                 :label :Private
                                 :model_provider :private
                                 :strategy :api_key}})
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :private/model
                                 :label "Private model"
                                 :model :model
                                 :provider :private}})
          nil
          {:fx setup-fx})}
