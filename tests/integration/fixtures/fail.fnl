{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [] (error :exploded) nil)})
          nil
          {:fx setup-fx})}
