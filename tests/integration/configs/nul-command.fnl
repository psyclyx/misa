(let [config {"providers" {"command" {"argv" ["bad\000arg"]}}}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.agent.stream))
  (app.include (. (require :tests.stock) :misa.providers.command))
  (app.define {:events {:request {:event :app/start
                                  :handler (fn []
                                             {:fx [{:type :provider.command
                                                    :id :invalid-command
                                                    :messages [{:role :user
                                                                :content [{:type :text
                                                                           :text :hello}]}]}]})}}})
  {:config config :definitions app.definitions})
