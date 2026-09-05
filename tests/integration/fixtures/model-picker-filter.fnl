{:setup (fn []
          (misa.reg_model {:id :picker/vendor/first
                           :label :First
                           :model :vendor/first
                           :provider :picker})
          (misa.reg_model {:id :picker/vendor/second
                           :label :Second
                           :model :vendor/second
                           :provider :picker})
          (misa.reg_fx :provider.picker
                       (fn [effect]
                         (assert (= effect.model :vendor/second))
                         {:event {:content [{:text (.. "picked " effect.model)
                                             :type :text}]
                                  :id effect.id
                                  :type :agent/result}
                          :type :dispatch}))
          nil)}

