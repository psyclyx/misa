(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                (let [definition {:id :openai
                                  :label :Private
                                  :model_provider :private
                                  :strategy :api_key}]
                  {:catalog :auth-providers
                   :id (. definition :id)
                   :value definition}))
  (table.insert declarations
                (let [definition {:id :private/model
                                  :label "Private model"
                                  :model :model
                                  :provider :private}]
                  {:catalog :models :id (. definition :id) :value definition}))
  nil
  (definitions.collect :tests.integration.fixtures.unavailable-models
    declarations
    {}))
