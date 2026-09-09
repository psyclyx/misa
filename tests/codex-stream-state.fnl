(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local definitions (require :misa.definitions))
(local specs ((fennel.dofile :extensions/misa/providers/openai-codex.fnl) {:config {}}))
(local application (misa.compose
 [{:definitions ((fennel.dofile :extensions/misa/json.fnl) {})}
  {:definitions ((fennel.dofile :extensions/misa/protocols/stream.fnl) {})}
  (misa.compose [{:definitions specs}])
  {:definitions (definitions :test [{:catalog :codex-records :id :test.metadata :value (fn [_ record] {:patch {:metadata record.value}})}])}]))
(misa._install application.definitions {:argv [] :config {}})
(local handlers (collect [_ entry (pairs specs.events)] entry.event entry.handler))
(local handler (. handlers :provider/openai-codex-complete))
(fn transition [db event]
  (local (before input) (values (misa.json.encode db) (misa.json.encode event)))
  (local result (handler db event))
  (assert (= before (misa.json.encode db)) "Codex adapter mutated prior state")
  (assert (= input (misa.json.encode event)) "Codex adapter mutated stream records")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(local original {:providers {:other {:active true}}})
(local initial (transition original {:id :request :phase :start}))
(assert (= original.providers.codex_streams nil))
(assert (= initial.providers.other original.providers.other))
(local records [{:type :response.output_text.delta :delta :text}
                {:type :response.reasoning_summary_text.delta :output_index 0 :delta :thinking}
                {:type :response.output_item.done :output_index 0
                 :item {:id :reasoning :type :reasoning :summary [{:text :thinking}]}}
                {:type :response.output_item.added :output_index 1
                 :item {:type :function_call :call_id :call :name :tool}}
                {:type :response.function_call_arguments.delta :output_index 1 :delta "{}"}
                {:type :response.output_item.done :output_index 1
                 :item {:type :function_call :call_id :call :name :tool :arguments "{}"}}
                {:type :response.completed :response {:usage {:input_tokens 10 :output_tokens 2
                                                             :input_tokens_details {:cached_tokens 3}}}}
                {:type :response.failed :response {:error {:message :failed}}}
                {:type :unknown}])
(local failure
       (G.for_all (G.vector (G.elements records))
                  (fn [batch]
                    (local (whole effects) (transition initial {:id :request :phase :data :records batch}))
                    (var split initial)
                    (local split-fx [])
                    (each [_ record (ipairs batch)]
                      (local (next fx) (transition split {:id :request :phase :data :records [record]}))
                      (set split next)
                      (each [_ effect (ipairs fx)] (table.insert split-fx effect)))
                    (assert (= (misa.json.encode whole) (misa.json.encode split)) "stream batching changed state")
                    (assert (= (misa.json.encode effects) (misa.json.encode (misa.stream.effects split-fx))) "stream batching changed effects"))
                  {:cases 1000 :size 25}))
(assert (not failure) (and failure (fennel.view failure)))
(local (reasoned reasoning-fx) (transition initial {:id :request :phase :data
                                                   :records [(. records 2) (. records 3)]}))
(assert (= (length reasoning-fx) 2) "reasoning summary was emitted after streamed reasoning")
(assert (= (. reasoning-fx 1 :event :delta :type) :thinking))
(assert (= (. reasoning-fx 2 :event :type) :agent/stream-state))
(local (completed completed-fx) (transition reasoned {:id :request :phase :data :records [(. records 7)]}))
(assert (= (. completed-fx 1 :event :usage :cache_read_tokens) 3))
(assert (= (. completed-fx 2 :type) :operation/finish))
(local (ended end-fx) (transition completed {:id :request :phase :end :ok true}))
(assert (= ended.providers.codex_streams.request nil))
(assert (= (. end-fx 1 :event :type) :agent/stream-end))
(local (failed error-fx) (transition initial {:id :request :phase :data :records [(. records 8)]}))
(assert (= (. error-fx 1 :event :message) :failed))
(local (failed-end failed-fx) (transition failed {:id :request :phase :end :ok true}))
(assert (= (length failed-fx) 0) "transport completion repeated an already emitted error")
(assert (= failed-end.providers.codex_streams.request nil))
(local (_ incomplete-fx) (transition initial {:id :request :phase :end :ok true}))
(assert (= (. incomplete-fx 1 :event :type) :agent/stream-error))

(assert (= (. (transition initial {:id :request :phase :data :records [{:type :test.metadata :value :custom}]})
              :providers :codex_streams :request :metadata) :custom))
;; Provider-state records separate reasoning segments during normalization.
(local (_ batched) (transition initial {:id :request :phase :data
                                        :records (fcollect [_ 1 32] {:type :response.output_text.delta :delta "x"})}))
(assert (= (length batched) 1))
(assert (= (. batched 1 :event :delta :text) (string.rep "x" 32)))
(local (_ boundaries) (transition initial {:id :request :phase :data
                                           :records [(. records 2) (. records 3) (. records 2)]}))
(assert (= (length boundaries) 3))
(assert (= (. boundaries 2 :event :type) :agent/stream-state))
(output "Codex stream state properties passed\n")
