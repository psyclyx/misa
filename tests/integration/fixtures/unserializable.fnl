{:setup (fn []
          (misa.reg_request_options_serializer :test.transport
                                               {:accepts (fn [name]
                                                           (= name :known))
                                                :serialize (fn [target
                                                                name
                                                                value]
                                                             (tset target name
                                                                   value)
                                                             true)})
          (misa.reg_model {:api {:request_options {:unknown {:default :selected}}
                                 :request_options_serializer :test.transport}
                           :id :test/model
                           :model :model
                           :provider :test})
          (misa.reg_fx :provider.test
                       (fn [] (error "blocked request reached provider") nil))
          nil)}

