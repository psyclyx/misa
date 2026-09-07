(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(misa._setup (fennel.dofile :extensions/json.fnl) {})
(local specs ((. (fennel.dofile :extensions/messages.fnl) :setup) {:config {:messages {:max_string 12}}}))
(local handlers {})
(var (register input-policy) nil)
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler))
  (when (= spec.type :register/interceptor) (set input-policy spec.value.before))
  (when (= spec.name :register/transcript-delta) (set register spec.handler)))
(fn initial [kind]
  (local blocks [{:id :block :kind kind :streaming true :chunks [] :byte_count 0
                 :argument_chunks [] :argument_bytes 0}])
  {:messages {:blocks blocks :by_response {:reply 1}
              :responses [{:id :reply :role :assistant :block_start 1 :block_count 1 :status :streaming
                           :started_monotonic_ms 100 :started_wall_ms 0}]}})
(fn transition [db fields]
  (local event (misa.patch fields {:type (or fields.type :transcript/block-delta) :response_id :reply :block_id :block}))
  (local before (misa.json.encode db))
  (local input (misa.json.encode event))
  (local result ((. handlers event.type) db event {:clock {:wall_ms 1000 :monotonic_ms 1100}
                                                 :terminal {:interactive true}}))
  (assert (= before (misa.json.encode db)) "delta mutated prior transcript")
  (assert (= input (misa.json.encode event)) "delta mutated provider event")
  (assert (not (and result result.db)))
  (local next (misa.patch db (or (and result result.patch) {})))
  (when (= event.type :transcript/block-delta)
    (assert (= next.messages.responses db.messages.responses)))
  (assert (= next.messages.transcript nil) "transcript blocks were duplicated")
  next)
(local failure
       (G.for_all (G.vector (G.elements [{:text :hello} {:text ""} {:text "世界"}
                                         {:arguments_json "{}"} {:arguments_json_delta "long argument fragment"}
                                         {:arguments_json_delta :x} {:arguments {:token :secret :x 1}}]))
                  (fn [events]
                    (each [_ kind (ipairs [:assistant :thinking :tool_call])]
                      (var db (initial kind))
                      (each [_ event (ipairs events)] (set db (transition db event)))
                      (local block (. db.messages.blocks 1))
                      (when (not= kind :tool_call)
                        (assert (= block.byte_count (length (table.concat block.chunks)))))))
                  {:cases 500 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(local text (initial :assistant))
(assert (= (transition text {:text ""}) text))
(local tool (initial :tool_call))
(local truncated (transition tool {:arguments_json_delta "abcdefghijklmnop"}))
(assert (. truncated.messages.blocks 1 :arguments_truncated))
(assert (= (transition truncated {:arguments_json_delta :ignored}) truncated))
(local replaced (transition truncated {:arguments_json "{}" :arguments_json_delta :ignored}))
(assert (= (. replaced.messages.blocks 1 :arguments_truncated) nil))
(assert (= (table.concat (. replaced.messages.blocks 1 :argument_chunks)) "{}"))
(local structured (transition tool {:arguments {:token :secret :x 1}}))
(assert (not= (. structured.messages.blocks 1 :arguments :token) :secret))
(register {:id :custom :value (fn [_ event] {:custom event.text})})
(assert (= (. (transition (initial :custom) {:text :value}) :messages :blocks 1 :custom) :value))
(local streamed (transition text {:text :hello}))
(local ended (transition streamed {:type :transcript/block-end}))
(assert (= (. ended.messages.blocks 1 :text) :hello))
(assert (= (. ended.messages.blocks 1 :chunks) nil))
(assert (= (. ended.messages.blocks 1 :streaming) false))
(local completed (transition streamed {:type :transcript/response-end :usage {:output_tokens 20}}))
(assert (= (. completed.messages.responses 1 :tokens_per_second) 20))
(assert (= (. completed.messages.responses 1 :metadata_block_id) :block))
(assert (= (. completed.messages.blocks 1 :text) :hello))
(local interrupted (transition streamed {:type :transcript/response-interrupted}))
(assert (. interrupted.messages.blocks 1 :interrupted))
(assert (= (. interrupted.messages.responses 1 :status) :interrupted))
(local final-tool (transition replaced {:type :transcript/block-end :arguments {:x 1} :name :tool :call_id :call}))
(assert (= (. final-tool.messages.blocks 1 :arguments :x) 1))
(assert (= (. final-tool.messages.blocks 1 :argument_chunks) nil))
(assert (= (. final-tool.messages.blocks 1 :argument_text) nil))
(local described (misa.patch tool {:messages {:blocks (misa.replace [{:id :block :kind :tool_call
                                                                    :streaming true :description :stale}])}}))
(local renamed (transition described {:type :transcript/block-end :name :missing}))
(assert (= (. renamed.messages.blocks 1 :description) nil) "renaming retained an old tool description")
(local finish-failure
       (G.for_all (G.tuple [(G.vector (G.elements [:a :b "世界" ""]))
                            (G.elements [:transcript/block-end :transcript/response-end :transcript/response-interrupted])])
                  (fn [sample]
                    (var db (initial :assistant))
                    (each [_ fragment (ipairs (. sample 1))] (set db (transition db {:text fragment})))
                    (local finished (transition db {:type (. sample 2)}))
                    (assert (= (. finished.messages.blocks 1 :text) (table.concat (. sample 1)))))
                  {:cases 500 :size 20}))
(assert (not finish-failure) (and finish-failure (fennel.view finish-failure)))
(local boot (transition {} {:type :app/start}))
(assert (= boot.messages.verbose false))
(local controls-failure
       (G.for_all (G.vector (G.elements [:messages/toggle-verbose :transcript/reset :messages/scroll]))
                  (fn [events]
                    (var db boot)
                    (var verbose false)
                    (each [_ type (ipairs events)]
                      (when (= type :messages/toggle-verbose) (set verbose (not verbose)))
                      (set db (transition db {: type :delta 3}))
                      (assert (= db.messages.verbose verbose))))
                  {:cases 500 :size 20}))
(assert (not controls-failure) (and controls-failure (fennel.view controls-failure)))
(local reset (transition (misa.patch streamed {:messages {:next_id 17 :top 5}}) {:type :transcript/reset}))
(assert (= reset.messages.next_id 17))
(assert (= reset.messages.top nil))
(assert (= (length reset.messages.blocks) 0))
(set misa.keybinding_action (fn [_ event] event.action))
(each [_ example (ipairs [{:kind :wheel_up :delta 3} {:kind :wheel_down :delta -3}
                          {:action :transcript_up :delta 12} {:action :transcript_down :delta -12}])]
  (local tx {:db boot :event {:type :terminal/input :kind example.kind :action example.action}
             :cofx {:terminal {:lines 24}}})
  (local before (misa.json.encode tx))
  (local next (input-policy tx))
  (assert (= before (misa.json.encode tx)) "transcript input policy mutated its transaction")
  (assert (= next.event.type :messages/scroll))
  (assert (= next.event.delta example.delta)))
(local response-started (transition boot {:type :transcript/response-start :role :assistant}))
(local block-started (transition response-started {:type :transcript/block-start :kind :assistant}))
(assert (= (. block-started.messages.responses 1 :block_count) 1))
(assert (= (. block-started.messages.blocks 1 :response_id) :reply))
(assert (= (. response-started.messages.responses 1 :block_count) 0))
(local complete-path (transition (transition block-started {:text :hello}) {:type :transcript/response-end}))
(assert (= (. complete-path.messages.blocks 1 :text) :hello))
(local creation-failure
       (G.for_all (G.vector (G.elements [:transcript/user :transcript/harness :transcript/tool-call
                                         :transcript/tool-result :transcript/assistant :transcript/reset]))
                  (fn [events]
                    (var db boot)
                    (each [_ type (ipairs events)]
                      (set db (transition db {: type :text :text :name :tool :id :call
                                             :content [{:type :text :text :assistant}]}))
                      (each [index owner (ipairs db.messages.responses)]
                        (assert (= (. db.messages.by_response owner.id) index))
                        (for [i owner.block_start (- (+ owner.block_start owner.block_count) 1)]
                          (assert (= (. db.messages.blocks i :response_id) owner.id))))))
                  {:cases 500 :size 20}))
(assert (not creation-failure) (and creation-failure (fennel.view creation-failure)))
(local called (transition boot {:type :transcript/tool-call :id :call :name :tool :arguments {:x 1}}))
(local returned (transition called {:type :transcript/tool-result :id :call :text :done}))
(assert (= (length returned.messages.blocks) 1))
(assert (= (. returned.messages.blocks 1 :result) :done))
(assert (= (. returned.messages.blocks 1 :status) :success))
(local interrupted-new (transition boot {:type :transcript/interrupted :request_id :new
                                         :content [{:type :text :text :partial}]}))
(assert (= (. interrupted-new.messages.responses 1 :status) :interrupted))
(assert (. interrupted-new.messages.blocks 1 :interrupted))
(output "transcript delta state properties passed\n")
