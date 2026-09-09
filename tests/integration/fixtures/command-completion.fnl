(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:description "test generic completion"
                                 :event :test/ping
                                 :name :/ping}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        (let [definition {:description "alternate completion"
                                 :event :test/pingpong
                                 :name :/pingpong}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        {:catalog :events  :value {:event :test/pingpong :handler (fn []
                                    {:fx [{:lines [{:spans [{:text :pongpong}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/ping :handler (fn []
                                    {:fx [{:lines [{:spans [{:text :pong}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.command-completion declarations {}))
