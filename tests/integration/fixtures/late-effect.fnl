{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:type :app/quit}
                                          {:type :not/native}]})})
          nil
          {:fx setup-fx})}
