(local definitions (require :misa.definitions))

;; The harness supplies a FIFO as python.so to pause native grammar loading.
(fn []
          (definitions :tests.syntax-worker [{:catalog :events  :value {:event :app/start :handler (fn [db]
                            {:patch {:done 0 :healthy 0}
                             :fx [{:type :syntax/highlight
                                   :id :blocked-grammar
                                   :language :python
                                   :source "answer = 42\n"
                                   :completion :fixture/highlighted
                                   :timeout_ms 60000}
                                  {:type :terminal/read}]})}}
                {:catalog :events  :value {:event :fixture/highlighted :handler (fn [db event]
                            (assert (= event.id :blocked-grammar))
                            (assert (= event.ok true))
                            (assert (= (type event.data) :table))
                            (assert (= (length event.data) 0)
                                    "invalid grammar did not fall back to plain text")
                            (assert (= event.source nil))
                            {:patch {:done 1}})}}
                {:catalog :events  :value {:event :terminal/input :handler (fn [db event]
                            (if (= event.kind :ctrl_d)
                                {:fx [{:type :app/quit}]}
                                {:patch {:healthy (when (= event.kind :tab) (+ db.healthy 1))}
                                 :fx [{:type :terminal/read}]}))}}
                {:catalog :views :id :main :value (fn [db]
                            {:lines [{:spans [{:text (.. :done= (or db.done 0)
                                                         " healthy="
                                                         (or db.healthy 0))}]}]})}] {}))
