{:setup (fn []
          (misa.reg_command {:completion :things
                             :description "generic command choice"
                             :event :test/choose
                             :name :/choose})
          (misa.reg_completion :things {:label :Alpha :value :alpha-one})
          (misa.reg_completion :things {:label :Beta :value :beta-two})
          (misa.reg_event :test/choose
                          (fn [_ event]
                            (assert (= event.arguments :beta-two))
                            {:fx [{:lines [{:spans [{:text event.arguments}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

