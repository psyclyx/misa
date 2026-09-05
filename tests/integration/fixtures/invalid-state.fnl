{:setup (fn []
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:completion :invalid/loaded
                                   :namespace :integration
                                   :type :state/load}]}))
          (misa.reg_event :invalid/loaded
                          (fn [_ event]
                            (assert (and (= event.ok false)
                                         (= event.message :InvalidState)))
                            {:fx [{:type :app/quit}]}))
          nil)}

