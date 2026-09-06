;; The harness supplies a FIFO as python.so to pause native grammar loading.
{:setup (fn []
          {:fx [{:type :register/event
                 :name :app/start
                 :handler (fn [db]
                            (set (db.done db.healthy) (values 0 0))
                            {: db
                             :fx [{:type :syntax/highlight
                                   :id :blocked-grammar
                                   :language :python
                                   :source "answer = 42\n"
                                   :completion :fixture/highlighted
                                   :timeout_ms 60000}
                                  {:type :terminal/read}]})}
                {:type :register/event
                 :name :fixture/highlighted
                 :handler (fn [db event]
                            (assert (= event.id :blocked-grammar))
                            (assert (= event.ok true))
                            (assert (= (type event.data) :table))
                            (assert (= (length event.data) 0)
                                    "invalid grammar did not fall back to plain text")
                            (assert (= event.source nil))
                            (set db.done 1)
                            {: db})}
                {:type :register/event
                 :name :terminal/input
                 :handler (fn [db event]
                            (if (= event.kind :ctrl_d)
                                {:fx [{:type :app/quit}]}
                                (do
                                  (when (= event.kind :tab)
                                    (set db.healthy (+ db.healthy 1)))
                                  {: db :fx [{:type :terminal/read}]})))}
                {:type :register/view
                 :handler (fn [db]
                            {:lines [{:spans [{:text (.. :done= (or db.done 0)
                                                         " healthy="
                                                         (or db.healthy 0))}]}]})}]})}
