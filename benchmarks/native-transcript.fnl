;; Exercise the default UI. Only input generation and initial transcript are fixtures.
{:setup (fn [context]
          (local count context.config.benchmark.blocks)
          (local mixed (= context.config.benchmark.scenario :mixed))
          (var previous nil)
          (var previous-responses nil)
          (var checked-highlights false)
          (local markdown "A **bold** [link](https://example.test) with 界 and é.\n\n- first\n- second\n\n> quoted text\n\n")
          (local source (if mixed (.. "```lua\nlocal answer = 42\n```\n\n" markdown) markdown))
          (local blocks (fcollect [index 1 count]
                          {:id (tostring index) :response_id :bench :kind :assistant :text source}))
          (local responses [])
          (local by-response {})
          (when mixed
            (local variants [{:kind :user :role :user :text "Review this change and explain the result."}
                             {:kind :thinking :role :assistant :text "1. Inspect the code\n2. Apply the change\n3. Verify the result"}
                             {:kind :tool_call :role :assistant :name :apply_patch :status :success
                              :arguments {:path :src/example.fnl :operation :update}
                              :result "@@ example @@\n- old value\n+ new value"}
                             {:kind :assistant :role :assistant :text source}])
            (each [index block (ipairs blocks)]
              (local variant (. variants (if (= index count) 4 (+ (% (- index 1) 4) 1))))
              (each [key value (pairs variant)] (tset block key value))
              (when (= block.kind :tool_call)
                (set block.text nil)
                (set block.call_id (.. :call/ index)))
              (local last (. responses (length responses)))
              (when (or (not last) (= variant.role :user) (= last.role :user))
                (local id (.. :bench/ index))
                (table.insert responses {: id :role variant.role :block_start index :block_count 0
                                         :started_wall_ms 0 :status :complete})
                (tset by-response id (length responses)))
              (local owner (. responses (length responses)))
              (set owner.block_count (+ owner.block_count 1))
              (when (and (= block.kind :assistant) (not= index count))
                (set owner.metadata_block_id block.id))
              (set block.response_id owner.id)
              (set block.started_wall_ms 0)))
          (when (not mixed)
            (table.insert responses {:id :bench :role :assistant :block_start 1 :block_count count
                                     :status :streaming})
            (set by-response.bench 1))
          (local tail-owner (. responses (length responses)))
          (set tail-owner.status :streaming)
          (tset blocks count {:id (tostring count) :response_id tail-owner.id :kind :assistant
                              :streaming true :chunks [source] :byte_count (length source)})
          ;; Validate fixture ownership before measuring any frame.
          (var covered 0)
          (each [index owner (ipairs responses)]
            (assert (= (. by-response owner.id) index))
            (assert (= owner.block_start (+ covered 1)))
            (when owner.metadata_block_id
              (assert (and (= owner.role :assistant) (= owner.status :complete)))
              (assert (= owner.metadata_block_id
                         (. blocks (- (+ owner.block_start owner.block_count) 1) :id))))
            (for [i owner.block_start (- (+ owner.block_start owner.block_count) 1)]
              (assert (= (. blocks i :response_id) owner.id)))
            (set covered (+ covered owner.block_count)))
          (assert (= covered count))
          (fn ready-check [] {:fx [{:type :dispatch :event {:type :bench/ready}}]})
          {:fx [{:type :register/model :value {:id :bench/model :model :model :provider :bench}}
                {:type :register/event-route
                 :value {:id :bench/input :event :terminal/input :priority 2000
                         :context [:db]
                         :resolve (fn [_ event]
                                    (when (and (= event.kind :alt)
                                               (or (= event.text :r) (= event.text :s) (= event.text :q)))
                                      {:type (if (= event.text :q) :bench/quit :bench/frame)
                                       :stream (= event.text :s)}))}}
                {:type :register/event :name :transcript/updated :handler ready-check}
                {:type :register/event :name :syntax/completed :handler ready-check}
                {:type :register/event :name :bench/ready
                 :handler (fn [db]
                            ;; Follow-up dispatch observes settled owner state in either setup order.
                            (when (and db.benchmark (= db.benchmark.step 0)
                                       (= db.editor.text "WAIT:0000")
                                       db.syntax (not (next db.syntax.pending)))
                              (var highlighted 0)
                              (each [_ entry (pairs db.syntax.documents)]
                                (each [_ slot (ipairs entry.slots)]
                                  (assert (and slot.done slot.data (> (length slot.data) 0))
                                          "mixed fixture requires successful native highlighting")
                                  (set highlighted (+ highlighted 1))))
                              (local expected (if mixed
                                                  (accumulate [n 0 _ block (ipairs blocks)]
                                                    (+ n (if (= block.kind :assistant) 1 0))) 0))
                              (assert (= highlighted expected) "missing highlighted documents")
                              (set checked-highlights true)
                              {:patch {:editor {:text "FRAME:0000" :cursor 10}}}))}
                {:type :register/event :name :app/start
                 :handler (fn [] {:fx [{:type :dispatch :event {:type :bench/seed}}]})}
                {:type :register/event :name :bench/quit
                 :handler (fn [] {:fx [{:type :app/quit}]})}
                {:type :register/event :name :bench/seed
                 :handler (fn []
                            {:patch {:messages {:blocks (misa.replace blocks) :by_response (misa.replace by-response)
                                                :responses (misa.replace responses) :verbose (when mixed true)}
                                     :editor {:text "WAIT:0000" :cursor 10}
                                     :benchmark {:step 0 :streamed 0}}
                             :fx [{:type :dispatch :event {:type :transcript/updated}}]})}
                {:type :register/event :name :bench/frame
                 :handler (fn [db event]
                            (assert (= (length db.messages.blocks) count))
                            (assert (or (not mixed) checked-highlights))
                            (for [index 1 (- count 1)]
                              (assert (or (not previous) (= (. db.messages.blocks index) (. previous index)))
                                      "unrelated block identity changed"))
                            (set previous db.messages.blocks)
                            (assert (or (not previous-responses) (= previous-responses db.messages.responses))
                                    "streaming changed response ownership")
                            (set previous-responses db.messages.responses)
                            (assert (not (next db.syntax.pending)) "initial highlighting has not settled")
                            (assert (= (table.concat (. db.messages.blocks count :chunks))
                                       (.. source (string.rep "x" db.benchmark.streamed)))
                                    "stream updates were lost")
                            (local step (+ db.benchmark.step 1))
                            {:patch {:benchmark {: step :streamed (+ db.benchmark.streamed (if event.stream 1 0))}
                                     :editor {:text (string.format "FRAME:%04d" step) :cursor 10}}
                             :fx (if event.stream
                                     [{:type :dispatch :event {:type :transcript/block-delta :response_id tail-owner.id
                                                               :block_id (tostring count) :text "x"}}
                                      {:type :terminal/read}]
                                     [{:type :terminal/read}])})}]})}
