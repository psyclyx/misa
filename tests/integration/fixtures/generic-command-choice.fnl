{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/command
                         :value {:completion :things
                                 :description "generic command choice"
                                 :event :test/choose
                                 :name :/choose}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :things
                         :value {:label :Alpha :value :alpha-one}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :things
                         :value {:label :Beta :value :beta-two}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/choose
                         :handler (fn [_ event]
                                    (assert (= event.arguments :beta-two))
                                    {:fx [{:lines [{:spans [{:text event.arguments}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
