{:setup (fn []
          (misa.reg_event :app/start
                          (fn [db]
                            (set db.ticks {:a 0 :b 0})
                            {: db
                             :fx [{:completion :tick/a
                                   :id :a
                                   :interval_ms 10
                                   :type :timer/start}
                                  {:completion :tick/b
                                   :id :b
                                   :interval_ms 10
                                   :type :timer/start}]}))

          (fn tick [db name]
            (tset db.ticks name (+ (. db.ticks name) 1))
            (if (and (> db.ticks.a 0) (> db.ticks.b 0))
                {: db
                 :fx [{:id :a :type :timer/stop}
                      {:id :b :type :timer/stop}
                      {:lines [{:spans [{:style {:foreground :default}
                                         :text :timers}]}]
                       :type :view/commit}
                      {:type :app/quit}]} {: db}))

          (misa.reg_event :tick/a (fn [db] (tick db :a)))
          (misa.reg_event :tick/b (fn [db] (tick db :b)))
          nil)}

