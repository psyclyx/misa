(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:completion :things
                                 :description "generic command choice"
                                 :event :test/choose
                                 :name :/choose}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        {:catalog :completions :id (.. :things "/" (. {:label :Alpha :value :alpha-one} :value)) :value {:group :things :value {:label :Alpha :value :alpha-one}}})
          (table.insert declarations
                        {:catalog :completions :id (.. :things "/" (. {:label :Beta :value :beta-two} :value)) :value {:group :things :value {:label :Beta :value :beta-two}}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/choose :handler (fn [_ event]
                                    (assert (= event.arguments :beta-two))
                                    {:fx [{:lines [{:spans [{:text event.arguments}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.generic-command-choice declarations {}))
