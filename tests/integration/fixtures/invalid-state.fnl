{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:completion :invalid/loaded
                                           :namespace :integration
                                           :type :state/load}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :invalid/loaded
                         :handler (fn [_ event]
                                    (assert (and (= event.ok false)
                                                 (= event.message :InvalidState)))
                                    {:fx [{:type :app/quit}]})})
          nil
          {:fx setup-fx})}
