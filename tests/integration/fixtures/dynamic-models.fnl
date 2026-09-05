{:setup (fn []
          (misa.reg_model {:context_window 10
                           :id :dynamic/old
                           :label :Old
                           :model :old
                           :provider :dynamic})
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:event {:authoritative true
                                           :models [{:context_window 20
                                                     :id :dynamic/new
                                                     :label :New
                                                     :model :new}]
                                           :provider :dynamic
                                           :type :models/replace-provider}
                                   :type :dispatch}]}))
          (misa.reg_fx :provider.dynamic
                       (fn [effect]
                         (assert (= effect.model :new))
                         {:event {:content [{:text "dynamic model" :type :text}]
                                  :id effect.id
                                  :type :agent/result}
                          :type :dispatch}))
          nil)}

