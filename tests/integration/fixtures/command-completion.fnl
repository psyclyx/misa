{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "test generic completion"
                                 :event :test/ping
                                 :name :/ping}})
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "alternate completion"
                                 :event :test/pingpong
                                 :name :/pingpong}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/pingpong
                         :handler (fn []
                                    {:fx [{:lines [{:spans [{:text :pongpong}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/ping
                         :handler (fn []
                                    {:fx [{:lines [{:spans [{:text :pong}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
