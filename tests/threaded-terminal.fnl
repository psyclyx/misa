;; The FIFO blocks a Fennel transaction until the PTY harness releases it.
;; loadfile is used only as a deterministic test gate; plugins use IO effects.
{:setup (fn [context]
          {:fx [{:type :register/event
                 :name :app/start
                 :handler (fn [db]
                            (set db.phase :BOOT)
                            (set db.count 0)
                            {: db :fx [{:type :terminal/read}]})}
                {:type :register/event
                 :name :terminal/input
                 :handler (fn [db event]
                            (match event.kind
                              :ctrl_d {:fx [{:type :app/quit}]}
                              :tab (do
                                     (set db.phase :READY)
                                     {: db :fx [{:type :terminal/read}]})
                              :ctrl_r (do
                                        ((assert (loadfile context.config.gate)))
                                        (set db.phase :RELEASED)
                                        {: db :fx [{:type :terminal/read}]})
                              (where kind
                                     (or (= kind :arrow_up)
                                         (= kind :arrow_down))) (do
                                                                                                (assert (= kind
                                                                                                           (if (= (% db.count
                                                                                                                     2)
                                                                                                                  0)
                                                                                                               :arrow_up
                                                                                                               :arrow_down))
                                                                                                        "buffered input reordered")
                                                                                                (set db.count
                                                                                                     (+ db.count
                                                                                                        1))
                                                                                                {: db
                                                                                                 :fx [{:type :terminal/read}]})
                              _ {:fx [{:type :terminal/read}]}))}
                {:type :register/view
                 :handler (fn [db cofx]
                            {:lines [{:spans [{:text (.. (or db.phase "")
                                                         " count="
                                                         (or db.count 0)
                                                         " size="
                                                         cofx.terminal.columns
                                                         :x cofx.terminal.lines)}]}]})}]})}
