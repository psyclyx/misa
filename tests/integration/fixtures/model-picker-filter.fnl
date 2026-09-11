(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                (let [definition {:id :picker/vendor/first
                                  :label :First
                                  :model :vendor/first
                                  :provider :picker}]
                  {:catalog :models :id (. definition :id) :value definition}))
  (table.insert declarations
                (let [definition {:id :picker/vendor/second
                                  :label :Second
                                  :model :vendor/second
                                  :provider :picker}]
                  {:catalog :models :id (. definition :id) :value definition}))
  (table.insert declarations
                {:catalog :effects
                 :id :provider.picker
                 :value (fn [effect]
                          (assert (= effect.model :vendor/second))
                          {:event {:content [{:text (.. "picked " effect.model)
                                              :type :text}]
                                   :id effect.id
                                   :type :agent/result}
                           :type :dispatch})})
  nil
  (definitions.collect :tests.integration.fixtures.model-picker-filter
    declarations
    {}))
