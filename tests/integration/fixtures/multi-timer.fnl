{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    {:patch {:ticks {:a 0 :b 0}}
                                     :fx [{:completion :tick/a
                                           :id :a
                                           :interval_ms 10
                                           :type :timer/start}
                                          {:completion :tick/b
                                           :id :b
                                           :interval_ms 10
                                           :type :timer/start}]})})

          (fn tick [db name]
            (local ticks (misa.patch db.ticks {name (+ (. db.ticks name) 1)}))
            (if (and (> ticks.a 0) (> ticks.b 0))
                {:patch {: ticks}
                 :fx [{:id :a :type :timer/stop}
                      {:id :b :type :timer/stop}
                      {:lines [{:spans [{:style {:foreground :default}
                                         :text :timers}]}]
                       :type :view/commit}
                      {:type :app/quit}]}
                {:patch {: ticks}}))

          (table.insert setup-fx
                        {:type :register/event
                         :name :tick/a
                         :handler (fn [db] (tick db :a))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :tick/b
                         :handler (fn [db] (tick db :b))})
          nil
          {:fx setup-fx})}
