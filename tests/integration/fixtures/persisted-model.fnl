(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (each [_ definition (ipairs [{:context_window 10
                                :id :persisted/declared
                                :label :Declared
                                :model :declared
                                :provider :persisted}
                               {:context_window 20
                                :id :persisted/chosen
                                :label :Chosen
                                :model :chosen
                                :provider :persisted}])]
    (table.insert declarations
                  {:catalog :models :id definition.id :value definition}))
  (table.insert declarations
                {:catalog :effects
                 :id :provider.persisted
                 :value (fn [effect]
                          {:event {:content [{:text (.. "used "
                                                        (tostring effect.model))
                                              :type :text}]
                                   :id effect.id
                                   :type :agent/result}
                           :type :dispatch})})
  nil
  (definitions.collect :tests.integration.fixtures.persisted-model declarations
    {}))
