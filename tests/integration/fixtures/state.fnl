{:setup (fn []
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:completion :state/loaded
                                   :namespace :integration
                                   :type :state/load}]}))
          (misa.reg_event :state/loaded
                          (fn [_ event]
                            (assert (= event.namespace :integration))
                            (if (= event.data misa.json_null)
                                {:fx [{:data {:count 7 :nested {:ok true}}
                                       :namespace :integration
                                       :type :state/save}
                                      {:type :app/quit}]}
                                (do
                                  (assert (and (= event.data.count 7)
                                               (= event.data.nested.ok true)))
                                  {:fx [{:lines [{:spans [{:text "state loaded"}]}]
                                         :type :view/commit}
                                        {:type :app/quit}]}))))
          nil)}

