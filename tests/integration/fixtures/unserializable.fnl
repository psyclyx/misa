{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/request-options-serializer
                         :id :test.transport
                         :serializer {:accepts (fn [name]
                                                 (= name :known))
                                      :serialize (fn [target name value]
                                                   (tset target name value)
                                                   true)}})
          (table.insert setup-fx
                        {:type :register/model
                         :value {:api {:request_options {:unknown {:default :selected}}
                                       :request_options_serializer :test.transport}
                                 :id :test/model
                                 :model :model
                                 :provider :test}})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.test
                         :handler (fn []
                                    (error "blocked request reached provider")
                                    nil)})
          nil
          {:fx setup-fx})}
