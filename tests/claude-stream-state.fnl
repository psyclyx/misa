(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local definitions (require :misa.definitions))
(local specs ((fennel.dofile :extensions/misa/providers/claude.fnl) {:config {}}))
(local application (misa.compose
 [{:definitions ((fennel.dofile :extensions/misa/json.fnl) {})}
  {:definitions ((fennel.dofile :extensions/misa/protocols/stream.fnl) {})}
  (misa.compose [{:definitions specs}])
  {:definitions (definitions :test [{:catalog :claude-records :id :custom :value (fn [state] {:state (misa.patch state {:custom true})})}
                         {:catalog :claude-stream-events :id :custom :value (fn [state] {:state (misa.patch state {:custom_partial true})})}])}]))
(misa._install application.definitions {:argv [] :config {}})
(local handlers (collect [_ entry (pairs specs.events)] entry.event entry.handler))
(fn transition [db records phase]
  (local event {:id :request :phase (or phase :data) : records :ok true})
  (local (before input) (values (misa.json.encode db) (misa.json.encode event)))
  (local result ((. handlers :provider/claude-complete) db event))
  (assert (= before (misa.json.encode db)) "Claude adapter mutated prior state")
  (assert (= input (misa.json.encode event)) "Claude adapter mutated records")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(local initial (transition {} [] :start))
(local start {:type :stream_event :event {:type :message_start :message {:id :message}}})
(local tool-start {:type :stream_event :event {:type :content_block_start :index 0
                                              :content_block {:type :tool_use :id :call :name :tool}}})
(local arguments {:type :stream_event :event {:type :content_block_delta :index 0
                                             :delta {:type :input_json_delta :partial_json "{\"x\":1}"}}})
(local final {:type :assistant :message {:id :message :content [{:type :tool_use :id :call :name :tool :input {:x 1}}]}})
(local result {:type :result :result :done})
(local pool [start tool-start arguments final
             {:type :rate_limit_event :rate_limit_info {:status :allowed_warning :rateLimitType :five_hour
                                                       :utilization 0.8 :resetsAt 1800000000}}
             {:type :stream_event :event {:type :message_delta :usage {:output_tokens 1}}}
             {:type :assistant :message {:id :other :content [{:type :text :text :fallback}]}}
             {:type :user :message {:content [{:type :tool_result :tool_use_id :call :content :done}]}}
             result])
(local failure
       (G.for_all (G.vector (G.elements pool))
                  (fn [batch]
                    (local (whole fx) (transition initial batch))
                    (var split initial)
                    (local collected [])
                    (each [_ record (ipairs batch)]
                      (local (next effects) (transition split [record]))
                      (set split next)
                      (each [_ effect (ipairs effects)] (table.insert collected effect)))
                    (assert (= (misa.json.encode whole) (misa.json.encode split)))
                    (assert (= (misa.json.encode fx) (misa.json.encode (misa.stream.effects collected))) "batching changed Claude output"))
                  {:cases 1000 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(fn tool-count [fx]
  (local indices {})
  (each [_ effect (ipairs fx)]
    (local value (and effect.event effect.event.delta))
    (when (and value (= value.type :tool_call)) (tset indices value.index true)))
  (accumulate [count 0 _ _ (pairs indices)] (+ count 1)))
(local (streamed fx) (transition initial [start tool-start arguments final final result]))
(assert (= (tool-count fx) 1))
(assert (= (. streamed.providers.claude_streams.request.tools :call :complete) true))
(local (_ reversed) (transition initial [final start tool-start arguments result]))
(assert (= (tool-count reversed) 1))
(local (_ fallback) (transition initial [start final result]))
(assert (= (tool-count fallback) 1) "metadata-only stream events suppressed final tools")
(local (_ texts) (transition initial [start
                                     {:type :stream_event :event {:type :content_block_delta :index 0
                                                                  :delta {:type :text_delta :text :hello}}}
                                     {:type :assistant :message {:id :message :content [{:type :text :text :hello}]}}
                                     {:type :assistant :message {:id :other :content [{:type :text :text :world}]}}
                                     result]))
(local content [])
(each [_ effect (ipairs texts)]
  (local value (and effect.event effect.event.delta))
  (when (and value (= value.type :text)) (table.insert content value.text)))
(assert (= (table.concat content) "helloworld"))
(local (_ metadata) (transition initial [start
                                        {:type :stream_event :event {:type :content_block_start :index 0
                                                                     :content_block {:type :thinking :thinking ""}}}
                                        {:type :stream_event :event {:type :content_block_delta :index 0
                                                                     :delta {:type :signature_delta :signature :opaque}}}
                                        {:type :assistant :message {:id :message :content [{:type :thinking :thinking :fallback}]}}]))
(assert (= (. metadata (length metadata) :event :delta :text) :fallback)
        "non-content records suppressed final fallback")
(local tool-result {:type :user :message {:content [{:type :tool_result :tool_use_id :call :content :done}]}})
(local (_ results) (transition initial [tool-result tool-result]))
(assert (= (length results) 1) "repeated tool results were emitted twice")
(assert (= (transition streamed pool) streamed) "records after result changed state")
(local (ended end-fx) (transition streamed [] :end))
(assert (= ended.providers.claude_streams.request nil))
(assert (= (. end-fx 1 :event :type) :agent/stream-end))
(local (failed failure-fx) (transition initial [{:type :result :is_error true :result :failed}]))
(assert (= (. failure-fx 1 :event :message) :failed))
(local (_ end-error-fx) (transition failed [] :end))
(assert (= (length end-error-fx) 0))
(local meta ((. handlers :models/provider-availability) initial {:provider :claude :subscription_type :max}))
(assert (= initial.providers.claude nil))
(assert (= (. (misa.patch initial meta.patch) :providers :claude :subscription_type) :max))

(local extended (transition initial [{:type :custom} {:type :stream_event :event {:type :custom}}]))
(assert extended.providers.claude_streams.request.custom)
(assert extended.providers.claude_streams.request.custom_partial)
;; CLI quota events are account facts, not response token counts or transcript text.
(each [_ utilization (ipairs [0 0.8 1 -1 2 misa.json-null "0.5"])]
  (local valid (and (= (type utilization) :number) (>= utilization 0) (<= utilization 1)))
  (local (_ quota-fx) (transition initial
                                 [{:type :rate_limit_event :uuid :private-id :session_id :private-session
                                   :rate_limit_info {:status :allowed_warning :rateLimitType :five_hour
                                                     : utilization :resetsAt 1800000000}}]))
  (assert (= (length quota-fx) 1))
  (local event (. quota-fx 1 :event))
  (assert (= event.type :provider/claude-quota))
  (local applied ((. handlers event.type) initial event))
  (local db (misa.patch initial applied.patch))
  (local quota db.providers.claude.usage)
  (local window (. quota.windows 1))
  (assert (= quota.unavailable (not valid)))
  (assert (= window.used (when valid (* utilization 100))))
  (assert (= window.remaining (when valid (- 100 (* utilization 100)))))
  (assert (= window.label "Claude · 5h"))
  (assert (= window.reset_at_unix 1800000000))
  (assert (= window.status :allowed_warning))
  (assert (= (. applied.fx 1 :event :type) :usage/updated))
  (local encoded (misa.json.encode quota))
  (assert (not (encoded:find "private" 1 true)))
  (local (_ unknown-fx) (transition initial [{:type :rate_limit_event :rate_limit_info {:status :allowed}}]))
  (local unknown-event (. unknown-fx 1 :event))
  (local cleared (misa.patch db (. ((. handlers unknown-event.type) db unknown-event) :patch)))
  (assert cleared.providers.claude.usage.unavailable)
  (assert (= (. cleared.providers.claude.usage.windows 1 :used) nil)))
(local (_ malformed-quota) (transition initial [{:type :rate_limit_event :rate_limit_info false}]))
(assert (= (length malformed-quota) 0))
(local (_ names) (transition initial
                             [{:type :assistant :message {:id :names :content
                                                         [{:type :tool_use :id :local :name :mcp__misa__read_file :input {}}
                                                          {:type :tool_use :id :foreign :name :mcp__other__read_file :input {}}]}}]))
(assert (= (. names 1 :event :delta :name) :read_file))
(assert (= (. names 2 :event :delta :name) :mcp__other__read_file))
(output "Claude stream state properties passed\n")
