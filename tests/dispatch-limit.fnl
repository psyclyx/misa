;; Each callback returns promptly; only the synchronous dispatch chain runs away.
{:setup (fn [context]
          {:fx [{:type :register/event
                 :name :app/start
                 :handler (fn [db]
                            (set (db.count db.notices db.healthy)
                                 (values 0 0 0))
                            {: db
                             :fx [{:type (if context.config.headless
                                             :dispatch
                                             :terminal/read)
                                   :event {:type :fixture/spin}}]})}
                {:type :register/event
                 :name :fixture/spin
                 :handler (fn [db]
                            (set db.count (+ db.count 1))
                            {: db
                             :fx [{:type :dispatch
                                   :event {:type :fixture/spin}}]})}
                {:type :register/event
                 :name :runtime/dispatch-limit
                 :handler (fn [db event]
                            (assert (= event.limit 8))
                            (assert (= event.event_type :fixture/spin))
                            (assert (and (= (type event.text) :string)
                                         (event.text:find "previously committed"
                                                          1 true)))
                            (set db.notices (+ db.notices 1))
                            {: db})}
                {:type :register/event
                 :name :terminal/input
                 :handler (fn [db event]
                            (match event.kind
                              :ctrl_d {:fx [{:type :app/quit}]}
                              :tab {:fx [{:type :dispatch
                                          :event {:type :fixture/spin}}]}
                              :arrow_down (do
                                            (set db.healthy (+ db.healthy 1))
                                            {: db :fx [{:type :terminal/read}]})
                              _ {:fx [{:type :terminal/read}]}))}
                {:type :register/view
                 :handler (fn [db]
                            {:lines [{:spans [{:text (.. :count=
                                                         (or db.count 0)
                                                         " notices="
                                                         (or db.notices 0)
                                                         " healthy="
                                                         (or db.healthy 0))}]}]})}]})}
