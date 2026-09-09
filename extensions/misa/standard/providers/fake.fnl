(local fake (require :misa.providers.fake))

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :fake) {}))

{:models {:fake/default {:id :fake/default
                         :label :Fake
                         :model :default
                         :provider :fake}}
 :serializers {:fake.options {:accepts (fn []
                                         true)
                              :serialize (fn [name value] {name value})}}
 :effects {:provider.fake (fn [effect]
                            (fake.request (settings) :fake.options effect))}
 :events {:provider.fake/respond {:event :provider/fake
                                  :handler (fn [db event]
                                             (fake.respond (. (settings)
                                                              :responses)
                                                           db event))}}}
