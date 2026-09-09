(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(app.define ((fennel.dofile :extensions/json.fnl) {}))
(local specs ((fennel.dofile :extensions/messages.fnl) {:config {:messages {:max_string 12}}}))
(local handlers {})
(local input-policy (. specs.routes :messages/global-keys :resolve))
(local blocks-for (. specs.services :transcript.blocks))
(each [_ spec (pairs specs.events)] (tset handlers spec.event spec.handler))
(app.define {:subscriptions specs.subscriptions :transcript-deltas specs.transcript-deltas
             :transcript-presentations specs.transcript-presentations})
(app.define {:transcript-deltas {:custom (fn [_ event] {:custom event.text})}
             :transcript-presentations {:custom (fn [model] {:role :custom.render :model {:custom model.id}})
                                        :custom.assistant (fn [] {:role :custom.assistant :model {}})}})
(app.install)
(set misa.components {})
(set misa.selection {})
(set misa.syntax {})
(set misa.costs {})
(local summary-fact (misa.sub {} [:messages/detail-indicator]))
(assert (= summary-fact.type :text))
(assert (= summary-fact.value :summary))
(assert (= summary-fact (misa.sub {:messages {:blocks []}} [:messages/detail-indicator])))
(assert (= (. (misa.sub {:messages {:verbose true}} [:messages/detail-indicator]) :value) :verbose))
(fn initial [kind]
  (local blocks [{:id :block :response_id :reply :kind kind :streaming true :chunks [] :byte_count 0
                 :argument_chunks [] :argument_bytes 0}])
  {:messages {:blocks blocks :next_id 0 :by_response {:reply 1}
              :responses [{:id :reply :role :assistant :block_start 1 :block_count 1 :status :streaming
                           :started_monotonic_ms 100 :started_wall_ms 0}]}})
(fn transition [db fields interactive]
  (local event (misa.patch fields {:type (or fields.type :transcript/block-delta) :response_id :reply :block_id :block}))
  (local before (misa.json.encode db))
  (local input (misa.json.encode event))
  (local result ((. handlers event.type) db event {:clock {:wall_ms 1000 :monotonic_ms 1100}
                                                 :terminal {:interactive (not= interactive false)}}))
  (assert (= before (misa.json.encode db)) "delta mutated prior transcript")
  (assert (= input (misa.json.encode event)) "delta mutated provider event")
  (assert (not (and result result.db)))
  (local next (misa.patch db (or (and result result.patch) {})))
  (when (= event.type :transcript/block-delta)
    (assert (= next.messages.responses db.messages.responses)))
  (assert (= next.messages.transcript nil) "transcript blocks were duplicated")
  (values next (or (and result result.fx) [])))
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

(assert (= (. (transition (initial :custom) {:text :value}) :messages :blocks 1 :custom) :value))
(local streamed (transition text {:text :hello}))
(local ended (transition streamed {:type :transcript/block-end}))
(assert (= (. ended.messages.blocks 1 :text) :hello))
(assert (= (. ended.messages.blocks 1 :chunks) nil))
(assert (= (. ended.messages.blocks 1 :streaming) false))
(local completed (transition streamed {:type :transcript/response-end :usage {:output_tokens 20}}))
(assert (= (. completed.messages.responses 1 :tokens_per_second) 20))
(assert (= (. completed.messages.responses 1 :metadata_block_id) nil))
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
(set misa.keybindings.action (fn [_ event] event.action))
(each [_ example (ipairs [{:kind :wheel_up :delta 3} {:kind :wheel_down :delta -3}
                          {:action :transcript_up :delta 12} {:action :transcript_down :delta -12}])]
  (local tx {:db boot :event {:type :terminal/input :kind example.kind :action example.action}
             :cofx {:terminal {:lines 24}}})
  (local before (misa.json.encode tx))
  (local next (input-policy tx.db tx.event tx.cofx))
  (assert (= before (misa.json.encode tx)) "transcript input policy mutated its transaction")
  (assert (= next.type :messages/scroll))
  (assert (= next.delta example.delta)))
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

;; Notifications are scoped to canonical owners and appended after prior effects.
(fn notification [db event response-id block-id interactive]
  (local (next fx) (transition db event interactive))
  (local last (. fx (length fx)))
  (assert (= last.type :dispatch))
  (assert (= last.event.type :transcript/updated))
  (assert (= last.event.response_id response-id))
  (assert (= last.event.block_id block-id))
  (var count 0)
  (each [_ effect (ipairs fx)]
    (when (and (= effect.type :dispatch) (= effect.event.type :transcript/updated))
      (set count (+ count 1))))
  (assert (= count 1) "mutation emitted duplicate transcript notifications")
  (values next fx))
(notification response-started {:type :transcript/block-start :kind :assistant} :reply :block)
(notification text {:text :new} :reply :block)
(notification tool {:arguments_json_delta "{}"} :reply :block)
(notification streamed {:type :transcript/block-end} :reply :block)
(notification streamed {:type :transcript/response-end} :reply nil)
(notification streamed {:type :transcript/response-interrupted} :reply nil)
(notification called {:type :transcript/tool-result :id :call :text :done} :transcript-1 :transcript-1/1)
(each [_ type (ipairs [:transcript/user :transcript/harness :transcript/tool-call :transcript/tool-result])]
  (notification boot {: type :text :standalone :id :call} :transcript-1 :transcript-1/1))
(notification boot {:type :transcript/assistant :request_id :legacy
                    :content [{:type :text :text :first} {:type :thinking :text :second}]} :legacy nil)
(notification boot {:type :transcript/interrupted :request_id :legacy
                    :content [{:type :text :text :partial}]} :legacy nil)
(notification streamed {:type :transcript/interrupted :request_id :reply} :reply nil)
(each [_ sample (ipairs [[boot {:type :transcript/response-start}]
                         [text {:text ""}]
                         [truncated {:arguments_json_delta :ignored}]
                         [boot {:type :transcript/assistant :content []}]
                         [boot {:type :transcript/response-interrupted}]
                         [response-started {:type :transcript/response-end}]])]
  (local (_ fx) (transition (. sample 1) (. sample 2)))
  (assert (= (length fx) 0) "empty/no-op lifecycle emitted a notification"))
(local combined (transition streamed {:type :transcript/user :text :other}))
(local before-blocks (misa.json.encode combined))
(assert (= (blocks-for combined nil) combined.messages.blocks) "all-block lookup copied the canonical array")
(local reply-blocks (blocks-for combined :reply))
(assert (= (length reply-blocks) 1))
(assert (= (. reply-blocks 1) (. combined.messages.blocks 1)))
(assert (= (. (blocks-for combined :transcript-1 :transcript-1/1) 1) (. combined.messages.blocks 2)))
(assert (= (length (blocks-for combined :reply :transcript-1/1)) 0))
(assert (= (length (blocks-for combined :missing)) 0))
(assert (= (length (blocks-for {} :missing)) 0))
(assert (= (length (blocks-for {:messages {}} :missing)) 0))
(assert (= (length (blocks-for {} nil)) 0))
(assert (= before-blocks (misa.json.encode combined)))
(assert (= (. combined.messages.blocks 1) (. streamed.messages.blocks 1)) "standalone creation replaced another owner's block")
(local multiple (notification combined {:type :transcript/assistant :request_id :multiple
                                        :content [{:type :text :text :first}
                                                  {:type :thinking :text :second}]} :multiple nil))
(local matched (blocks-for multiple :multiple))
(assert (= (length matched) 2))
(assert (= (. matched 1) (. multiple.messages.blocks 3)))
(assert (= (. matched 2) (. multiple.messages.blocks 4)))
(local delta (notification multiple {:text :more} :reply :block))
(assert (= (. delta.messages.blocks 2) (. multiple.messages.blocks 2)))
(assert (= (. delta.messages.blocks 3) (. multiple.messages.blocks 3)))
(assert (= (. delta.messages.blocks 4) (. multiple.messages.blocks 4)))
(local finalized (notification multiple {:type :transcript/response-end} :reply nil))
(assert (= (. finalized.messages.blocks 3) (. multiple.messages.blocks 3)))
(local committed-lines [{:spans [{:text :committed}]}])
(set misa.components.render (fn [] {:lines committed-lines}))
(each [_ sample (ipairs [[boot {:type :transcript/user :text :user} :transcript-1 :transcript-1/1]
                         [boot {:type :transcript/harness :text :harness} :transcript-1 :transcript-1/1]
                         [boot {:type :transcript/assistant :request_id :legacy
                                :content [{:type :text :text :answer}]} :legacy]
                         [streamed {:type :transcript/response-end} :reply]])]
  (local (_ fx) (notification (. sample 1) (. sample 2) (. sample 3) (. sample 4) false))
  (assert (= (length fx) 2))
  (assert (= (. fx 1 :type) :view/commit))
  (assert (= (. fx 1 :lines) committed-lines)))
(set misa.components.render nil)
(local project (. specs.projections :transcript.project :render))

(local cached {:lines [{:spans [{:text :cached}]}]})
(var rendered-role nil)
(var rendered-model nil)
(set misa.components.project (fn [_ _ items]
                              {:views (icollect [_ item (ipairs items)]
                                        (if item.chrome {:lines []}
                                          (do (set rendered-role item.role)
                                            (set rendered-model item.model)
                                            cached)))}))
(set misa.selection.decorate (fn [] [{:spans [{:text :decorated}]}]))
(set misa.selection.state nil)
(set misa.syntax.for-model nil)
(set misa.costs.response nil)
(fn render [db]
  (local before (misa.json.encode db))
  (local cache-before (misa.json.encode cached))
  (local context {:columns 80})
  (local result (project db context))
  (assert (= before (misa.json.encode db)) "presentation mutated transcript facts")
  (assert (= cache-before (misa.json.encode cached)) "decoration mutated cached component output")
  (assert (= context.markdown nil) "presentation mutated render context")
  result)
(assert (= (. (render streamed) 1 :spans 1 :text) :decorated))
(assert (= rendered-role :transcript.assistant))
(assert (= rendered-model.rail :rail.assistant))
(local thinking (initial :thinking))
(render thinking)
(assert (= rendered-role :transcript.thinking))
(local collapsed (transition thinking {:type :transcript/block-end}))
(render collapsed)
(assert (= rendered-role :transcript.thinking_collapsed))
(render (misa.patch collapsed {:messages {:verbose true}}))
(assert (= rendered-role :transcript.thinking))
(render returned)
(assert (= rendered-model.collapsed true))
(assert (= rendered-model.selection_text nil))
(set misa.selection.state (fn [] {:id "12:transcript-1transcript-1/1" :text :selected :first 0 :last 8}))
(render returned)
(assert (= rendered-model.selection_source :result))
(assert (= rendered-model.selection_text :selected))
(assert (= rendered-model.collapsed false))

(render (initial :custom))
(assert (= rendered-role :custom.render))
(assert (= rendered-model.custom :block))

;; SDK result echoes update the original section instead of appending duplicates.
(local tool-db (misa.patch (initial :tool_call)
                           {:messages {:blocks (misa.replace [{:id :block :response_id :reply
                                                              :kind :tool_call :call_id :call}])}}))
(local first-result (transition tool-db {:type :transcript/tool-result :id :call :text :ok}))
(local repeated-result (transition first-result {:type :transcript/tool-result :id :call :text :ok}))
(assert (= (length repeated-result.messages.blocks) 1))
(assert (= (. repeated-result.messages.blocks 1 :result) :ok))

;; A response owns the display metadata even when its only content is a tool.
(var projected-items nil)
(set misa.selection.state nil)
(set misa.components.project
     (fn [_ _ items]
       (set projected-items items)
       {:views (icollect [_ item (ipairs items)] {:lines [{:spans [{:text item.role}]}]})}))
(set misa.costs.response (fn [_ id] {:type :money :amount 0.25 :currency :USD :response_id id}))
(local tool-completed (transition tool-db {:type :transcript/response-end :usage {:output_tokens 20}}))
(render tool-completed)
(assert (= (length projected-items) 3))
(assert (= (. projected-items 1 :role) :transcript.group_header))
(assert (= (. projected-items 2 :role) :transcript.tool_call))
(assert (= (. projected-items 3 :role) :transcript.group_footer))
(local first-owner (. tool-completed.messages.responses 1))
(local second-owner (misa.patch first-owner {:id :followup :block_start 2 :block_count 1}))
(local turn-db (misa.patch tool-completed {:messages
                  {:responses (misa.replace [first-owner second-owner])
                   :by_response { :followup 2}
                   :blocks (misa.replace [(. tool-completed.messages.blocks 1)
                                         {:id :answer :kind :assistant :role :assistant
                                          :response_id :followup :text :done}])}}))
(render turn-db)
(assert (= (length projected-items) 4) "tool continuation introduced another turn divider")
(assert (= (. projected-items 4 :role) :transcript.group_footer))
(assert (= (. projected-items 4 :model :cost :amount) 0.5) "turn cost did not include both responses")
(assert (= (. projected-items 4 :model :tokens_per_second) nil) "per-request rate leaked into multi-request turn")
(render tool-completed)
(assert (= (. projected-items 3 :model :cost :amount) 0.25))
(assert (= (. projected-items 3 :model :elapsed_ms) 1000))
(assert (= (. projected-items 3 :model :tokens_per_second) 20))
(assert (= (. projected-items 2 :model :cost) nil))
(assert (= (. projected-items 2 :model :tokens_per_second) nil))
(project tool-completed {:columns 80 :document_id "5:replyblock"})
(assert (= (length projected-items) 1) "selection geometry included group chrome")
(assert (= (. projected-items 1 :role) :transcript.tool_call))

;; Execution timing is independent of generation and stable under result echoes.
(local timed-tool (transition tool-db {:type :transcript/tool-start :id :call}))
(assert (= (. timed-tool.messages.blocks 1 :execution_started_monotonic_ms) 1100))
(local timed-result ((. handlers :transcript/tool-result) timed-tool
                    {:type :transcript/tool-result :id :call :text :ok}
                    {:clock {:wall_ms 5000 :monotonic_ms 3600}}))
(local timed-db (misa.patch timed-tool timed-result.patch))
(assert (= (. timed-db.messages.blocks 1 :elapsed_ms) 2500))
(assert (= (. timed-db.messages.responses 1 :started_monotonic_ms) 100))
(local echoed ((. handlers :transcript/tool-result) timed-db
               {:type :transcript/tool-result :id :call :text :ok}
               {:clock {:wall_ms 9000 :monotonic_ms 7600}}))
(assert (= (. (misa.patch timed-db echoed.patch) :messages :blocks 1 :elapsed_ms) 2500))

;; The provider may only announce its stream after substantial request latency.
(local empty {:messages {:blocks [] :responses [] :by_response {} :next_id 0}})
(local request-start ((. handlers :transcript/response-start) empty
                      {:response_id :delayed :started_wall_ms 900 :started_monotonic_ms 100}
                      {:clock {:wall_ms 3900 :monotonic_ms 3100}}))
(local delayed (misa.patch empty request-start.patch))
(assert (= (. delayed.messages.responses 1 :started_monotonic_ms) 100))
(assert (= (. delayed.messages.responses 1 :started_wall_ms) 900))
(local request-end ((. handlers :transcript/response-end) delayed
                    {:response_id :delayed :usage {:output_tokens 80}}
                    {:clock {:wall_ms 4900 :monotonic_ms 4100} :terminal {:interactive true}}))
(assert (= (. (misa.patch delayed request-end.patch) :messages :responses 1 :elapsed_ms) 4000))
(assert (= (. (misa.patch delayed request-end.patch) :messages :responses 1 :tokens_per_second) 20))

;; Group duration includes executions without adding concurrent tool times.
(local executing (transition tool-completed {:type :transcript/tool-start :id :call}))
(render executing)
(assert (= (. projected-items 1 :model :status) :running))
(assert (= (. projected-items 3 :model :elapsed_ms) nil) "active tool showed a final group duration")
(assert (= (. projected-items 3 :model :tokens_per_second) 20))
(local completed-execution ((. handlers :transcript/tool-result) executing
                           {:type :transcript/tool-result :id :call :text :ok}
                           {:clock {:wall_ms 5000 :monotonic_ms 3600}}))
(local executed (misa.patch executing completed-execution.patch))
(render executed)
(assert (= (. projected-items 1 :model :status) :complete))
(assert (= (. projected-items 3 :model :elapsed_ms) 3500))
(assert (= (. projected-items 3 :model :tokens_per_second) 20))
(local parallel (misa.patch executed
                            {:messages {:responses (misa.replace [(misa.patch (. executed.messages.responses 1) {:block_count 2})])
                                        :blocks (misa.replace [(. executed.messages.blocks 1)
                                                              {:id :parallel :response_id :reply :kind :tool_call
                                                               :status :success :result :ok
                                                               :execution_started_monotonic_ms 1100
                                                               :execution_completed_monotonic_ms 4600 :elapsed_ms 3500}])}}))
(render parallel)
(assert (= (. projected-items 4 :model :elapsed_ms) 4500) "parallel durations were added")
(assert (= (. projected-items 4 :model :tokens_per_second) 20))


;; Standalone result selection uses frozen text even after a live result update.
(local standalone-result (misa.patch (initial :tool_result)
                                     {:messages {:blocks (misa.replace [{:id :block :response_id :reply
                                                                        :kind :tool_result :text :old :result :latest}])}}))
(set misa.selection.state (fn [] {:id "5:replyblock" :text :frozen :first 0 :last 6}))
(render standalone-result)
(local selected-result (. projected-items 2 :model))
(assert (= selected-result.selection_source :result))
(assert (= selected-result.selection_text :frozen))
(assert (= selected-result.collapsed false))
(local override-specs ((fennel.dofile :extensions/messages.fnl)
                      {:config {:messages {:presentations {:assistant :custom.assistant}}}}))
(local override-project (. override-specs.projections :transcript.project :render))


(override-project streamed {:columns 80})
(assert (= (. projected-items 2 :role) :custom.assistant))
(override-project (initial :thinking) {:columns 80})
(assert (= (. projected-items 2 :role) :transcript.thinking) "override discarded other default presentations")
(output "transcript delta state properties passed\n")
