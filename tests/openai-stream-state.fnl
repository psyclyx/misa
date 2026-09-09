(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local definitions (require :misa.definitions))
(local protocol (require :misa.protocols.openai))
(local specs (protocol.configure {:id :test :url "https://example.invalid/chat"
                                   :models [] :models_url "https://example.invalid/models"
                                   :models_credential false :credential :test}))
(local application (misa.compose
 [{:definitions ((fennel.dofile :extensions/misa/json.fnl) {})}
  {:definitions ((fennel.dofile :extensions/misa/agent/stream.fnl) {})}
  (misa.compose [{:definitions protocol.definitions} {:definitions specs}])
  {:definitions (definitions :test [{:catalog :openai-deltas :id :custom :value (fn [delta] (when delta.custom [{:type :text :text delta.custom}]))}])}]))
(misa._install application.definitions {:argv [] :config {}})
(local handlers (collect [_ entry (pairs specs.events)] entry.event entry.handler))
(local discovery ((. handlers :models/discover) {} {:provider :test}))
(assert (= (. discovery.fx 1 :credential) nil) "disabled discovery credentials were still attached")
(fn transition [db event]
  (local (before input) (values (misa.json.encode db) (misa.json.encode event)))
  (local result ((. handlers :provider/test-complete) db event))
  (assert (= before (misa.json.encode db)) "OpenAI adapter mutated state")
  (assert (= input (misa.json.encode event)) "OpenAI adapter mutated records")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(local original {:providers {:other {:active true}}})
(local initial (transition original {:id :request :phase :start}))
(assert (= initial.providers.openai_streams.request false))
(assert (= original.providers.openai_streams nil))
(assert (= initial.providers.other original.providers.other))
(local parallel (transition initial {:id :second :phase :start}))
(local first-finished (transition parallel {:id :request :phase :data :terminal true}))
(assert (= first-finished.providers.openai_streams.second false))
(assert (= parallel.providers.openai_streams.request false))
(local records [{:choices [{:delta {:content :hello}}]}
                {:choices [{:delta {:reasoning_content :thinking}}]}
                {:choices [{:delta {:reasoning :alternate}}]}
                {:choices [{:delta {:tool_calls [{:index 0 :id :call :function {:name :tool :arguments "{"}}
                                                 {:index 1 :id :other :function {:name :second :arguments "{}"}}]}}]}
                {:choices [{:delta {:tool_calls [{:index 0 :function {:arguments "}"}}]}}]}
                {:choices [{:finish_reason :tool_calls}]}
                {:usage {:prompt_tokens 10 :completion_tokens 2 :prompt_tokens_details {:cached_tokens 3}}}
                {:choices []}])
(local failure
       (G.for_all (G.vector (G.elements records))
                  (fn [batch]
                    (local (whole fx) (transition initial {:id :request :phase :data :records batch}))
                    (var split initial)
                    (local collected [])
                    (each [_ record (ipairs batch)]
                      (local (next effects) (transition split {:id :request :phase :data :records [record]}))
                      (set split next)
                      (each [_ effect (ipairs effects)] (table.insert collected effect)))
                    (assert (= (misa.json.encode whole) (misa.json.encode split)))
                    (assert (= (misa.json.encode fx) (misa.json.encode (misa.stream.effects collected))) "record batching changed output"))
                  {:cases 1000 :size 25}))
(assert (not failure) (and failure (fennel.view failure)))
(local (_ tool-fx) (transition initial {:id :request :phase :data :records [(. records 4) (. records 5)]}))
(assert (= (. tool-fx 1 :event :delta :index) 0))
(assert (= (. tool-fx 2 :event :delta :index) 1))
(assert (= (. tool-fx 3 :event :delta :arguments_json_delta) "}"))
(local (terminal terminal-fx) (transition initial {:id :request :phase :data :terminal true :records [(. records 7)]}))
(assert (= terminal.providers.openai_streams.request true))
(assert (= (. terminal-fx 1 :event :usage :cache_read_tokens) 3))
(assert (= (. terminal-fx 2 :type) :operation/finish))
(local (unchanged ignored) (transition terminal {:id :request :phase :data :terminal true :records records}))
(assert (= unchanged terminal))
(assert (= (length ignored) 0))
(local (ended ended-fx) (transition terminal {:id :request :phase :end :ok true}))
(assert (= ended.providers.openai_streams.request nil))
(assert (= (. ended-fx 1 :event :type) :agent/stream-end))
(local (_ missing-fx) (transition initial {:id :request :phase :end :ok true}))
(assert (= (. missing-fx 1 :event :message) "OpenAI stream ended without [DONE]"))
(local (_ http-fx) (transition initial {:id :request :phase :end :ok false :status 429 :body :limited}))
(assert (= (. http-fx 1 :event :message) "HTTP 429: limited"))

(local (_ custom-fx) (transition initial {:id :request :phase :data :records [{:choices [{:delta {:custom :custom}}]}]}))
(assert (= (. custom-fx 1 :event :delta :text) :custom))
(output "OpenAI stream state properties passed\n")
