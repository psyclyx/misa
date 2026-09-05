{:setup (fn []
          (misa.reg_command {:description "test generic completion"
                             :event :test/ping
                             :name :/ping})
          (misa.reg_event :test/ping
                          (fn []
                            {:fx [{:lines [{:spans [{:text :pong}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

