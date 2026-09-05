{:setup (fn []
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:type :app/quit} {:type :not/native}]}))
          nil)}

