;; Exercise the default UI. Only input generation and initial transcript are fixtures.
{:setup (fn [context]
          (local count context.config.benchmark.blocks)
          (var previous nil)
          (local source "A **bold** [link](https://example.test) with 界 and é.\n\n- first\n- second\n\n> quoted text\n\n")
          (local blocks (fcollect [index 1 count]
                          {:id (tostring index) :response_id :bench :kind :assistant :text source}))
          (tset blocks count {:id (tostring count) :response_id :bench :kind :assistant
                              :streaming true :chunks [source] :byte_count (length source)})
          {:fx [{:type :register/model :value {:id :bench/model :model :model :provider :bench}}
                {:type :register/interceptor
                 :value {:id :bench/input
                         :before (fn [tx]
                                   (if (and (= tx.event.type :terminal/input) (= tx.event.kind :alt)
                                            (or (= tx.event.text :r) (= tx.event.text :s) (= tx.event.text :q)))
                                       (misa.patch tx {:event (misa.replace {:type (if (= tx.event.text :q) :bench/quit :bench/frame)
                                                                            :stream (= tx.event.text :s)})})
                                       tx))}}
                {:type :register/event :name :app/start
                 :handler (fn [] {:fx [{:type :dispatch :event {:type :bench/seed}}]})}
                {:type :register/event :name :bench/quit
                 :handler (fn [] {:fx [{:type :app/quit}]})}
                {:type :register/event :name :bench/seed
                 :handler (fn []
                            {:patch {:messages {:blocks (misa.replace blocks) :by_response {:bench 1}
                                                :responses (misa.replace [{:id :bench :role :assistant
                                                                          :block_start 1 :block_count count :status :streaming}])}
                                     :editor {:text "FRAME:0000" :cursor 10}
                                     :benchmark {:step 0 :streamed 0}}})}
                {:type :register/event :name :bench/frame
                 :handler (fn [db event]
                            (assert (= (length db.messages.blocks) count))
                            (for [index 1 (- count 1)]
                              (assert (or (not previous) (= (. db.messages.blocks index) (. previous index)))
                                      "unrelated block identity changed"))
                            (set previous db.messages.blocks)
                            (assert (= (table.concat (. db.messages.blocks count :chunks))
                                       (.. source (string.rep "x" db.benchmark.streamed)))
                                    "stream updates were lost")
                            (local step (+ db.benchmark.step 1))
                            {:patch {:benchmark {: step :streamed (+ db.benchmark.streamed (if event.stream 1 0))}
                                     :editor {:text (string.format "FRAME:%04d" step) :cursor 10}}
                             :fx (if event.stream
                                     [{:type :dispatch :event {:type :transcript/block-delta :response_id :bench
                                                               :block_id (tostring count) :text "x"}}
                                      {:type :terminal/read}]
                                     [{:type :terminal/read}])})}]})}
