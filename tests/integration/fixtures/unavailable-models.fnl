{:setup (fn []
          (misa.reg_auth_provider {:id :openai
                                   :label :Private
                                   :model_provider :private
                                   :strategy :api_key})
          (misa.reg_model {:id :private/model
                           :label "Private model"
                           :model :model
                           :provider :private})
          nil)}

