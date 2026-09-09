(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :serializers :id :test.transport :value {:accepts (fn [name]
                                                 (= name :known))
                                      :serialize (fn [name value] {name value})}})
          (table.insert declarations
                        (let [definition {:api {:request_options {:unknown {:default :selected}}
                                       :request_options_serializer :test.transport}
                                 :id :test/model
                                 :model :model
                                 :provider :test}] {:catalog :models :id (. definition :id) :value definition}))
          (table.insert declarations
                        {:catalog :effects :id :provider.test :value (fn []
                                    (error "blocked request reached provider")
                                    nil)})
          nil
          (definitions :tests.integration.fixtures.unserializable declarations {}))
