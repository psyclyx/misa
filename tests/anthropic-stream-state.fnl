(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local definitions (require :misa.definitions))
(local protocol (require :protocol.anthropic))
(local specs (protocol.configure {:id :test :url "https://example.invalid/messages" :models []}))
(local application (misa.compose
 [{:definitions ((fennel.dofile :extensions/json.fnl) {})}
  {:definitions ((fennel.dofile :extensions/stream.fnl) {})}
  (misa.compose [{:definitions protocol.definitions} {:definitions specs}])
  {:definitions (definitions :test [{:catalog :anthropic-block-deltas :id :custom :value (fn [_ record] {:patch {:custom record.delta.value}})}
                         {:catalog :anthropic-block-starts :id :custom :value (fn [] {:patch {:custom_started true}})}
                         {:catalog :anthropic-records :id :custom :value (fn [] {:patch {:custom_record true}})}])}]))
(misa._install application.definitions {:argv [] :config {}})
(local handlers (collect [_ entry (pairs specs.events)] entry.event entry.handler))
(local handler (. handlers :provider/test-complete))
(fn transition [db event]
  (local (before input) (values (misa.json.encode db) (misa.json.encode event)))
  (local result (handler db event))
  (assert (= before (misa.json.encode db)) "Anthropic adapter mutated state")
  (assert (= input (misa.json.encode event)) "Anthropic adapter mutated records")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(local original {:providers {:other {:active true}}})
(local initial (transition original {:id :request :phase :start}))
(assert (= original.providers.anthropic_streams nil))
(assert (= initial.providers.other original.providers.other))
(local records [{:type :content_block_start :index 0 :content_block {:type :thinking :thinking :plan :signature :sig}}
                {:type :content_block_delta :index 0 :delta {:type :thinking_delta :thinking " next"}}
                {:type :content_block_delta :index 0 :delta {:type :signature_delta :signature :nature}}
                {:type :content_block_stop :index 0}
                {:type :content_block_start :index 1 :content_block {:type :redacted_thinking :data :opaque}}
                {:type :content_block_start :index 2 :content_block {:type :tool_use :id :call :name :tool}}
                {:type :content_block_delta :index 2 :delta {:type :input_json_delta :partial_json "{}"}}
                {:type :content_block_delta :index 3 :delta {:type :text_delta :text :answer}}
                {:type :message_start :message {:usage {:input_tokens 10 :cache_read_input_tokens 2
                                                       :cache_creation_input_tokens 3}}}
                {:type :message_delta :usage {:output_tokens 4} :delta {:stop_reason :end_turn}}
                {:type :message_stop}
                {:type :error :error {:message :failed}}
                {:type :unknown}])
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
(local begun (transition initial {:id :request :phase :data :records [(. records 1)]}))
(local (signed signed-fx) (transition begun {:id :request :phase :data :records [(. records 2) (. records 3) (. records 4)]}))
(assert (= (. begun.providers.anthropic_streams.request.blocks "0" :signature) :sig))
(assert (= (next signed.providers.anthropic_streams.request.blocks) nil))
(local value (. signed-fx 2 :event :value))
(assert (= value.signature :signature))
(assert (= value.thinking "plan next"))
(local replay (misa.protocols.anthropic-messages
               [{:role :assistant :content [{:type :thinking :text :discarded}]
                 :provider_state [{:provider :test : value}]}] :test))
(assert (= (. replay 1 :content 1) value))
(assert (= (length (. replay 1 :content)) 1))
(local (_ opaque-fx) (transition signed {:id :request :phase :data :records [(. records 5)]}))
(assert (= (. opaque-fx 1 :event :value :data) :opaque))
(local (_ tool-fx) (transition initial {:id :request :phase :data :records [(. records 6) (. records 7)]}))
(assert (= (. tool-fx 1 :event :delta :id) :call))
(assert (= (. tool-fx 2 :event :delta :arguments_json_delta) "{}"))
(local parallel (transition signed {:id :second :phase :start}))
(local (stopped stop-fx) (transition parallel {:id :request :phase :data :records [(. records 11)]}))
(assert (= stopped.providers.anthropic_streams.second parallel.providers.anthropic_streams.second))
(assert (= (. stop-fx 1 :type) :operation/finish))
(assert (= (transition stopped {:id :request :phase :data :records records}) stopped))
(local (ended end-fx) (transition stopped {:id :request :phase :end :ok true}))
(assert (= ended.providers.anthropic_streams.request nil))
(assert (= (. end-fx 1 :event :type) :agent/stream-end))
(local failed (transition initial {:id :request :phase :data :records [(. records 12)]}))
(local (_ failed-fx) (transition failed {:id :request :phase :end :ok true}))
(assert (= (length failed-fx) 0) "transport completion repeated the provider error")
(local (_ mismatch-fx) (transition initial {:id :request :phase :end :ok false :message :CredentialProfileMismatch}))
(assert (= (. mismatch-fx 2 :event :provider) :test))
(assert (= (. mismatch-fx 2 :event :reason) :relogin-required))
(assert (= (. mismatch-fx 2 :event :available) false))
(local (_ incomplete-fx) (transition initial {:id :request :phase :end :ok true}))
(assert (= (. incomplete-fx 1 :event :message) "Anthropic stream ended without message_stop"))
(local (_ http-fx) (transition initial {:id :request :phase :end :ok false :status 429 :body :limited}))
(assert (= (. http-fx 1 :event :message) "HTTP 429: limited"))
(local (_ usage-fx) (transition initial {:id :request :phase :data :records [(. records 9) (. records 10)]}))
(assert (= (. usage-fx 1 :event :usage :cache_read_tokens) 2))
(assert (= (. usage-fx 1 :event :usage :cache_write_tokens) 3))
(assert (= (. usage-fx 1 :event :usage :input_includes_cache) false))
(assert (= (. usage-fx 2 :event :usage :output_tokens) 4))
(assert (= (. usage-fx 2 :event :stop_reason) :end_turn))

(assert (= (. (transition initial {:id :request :phase :data
                                  :records [{:type :content_block_delta :delta {:type :custom :value :test}}]})
              :providers :anthropic_streams :request :custom) :test))
(local extended (transition initial {:id :request :phase :data
                                    :records [{:type :content_block_start :content_block {:type :custom}}
                                              {:type :custom}]}))
(assert extended.providers.anthropic_streams.request.custom_started)
(assert extended.providers.anthropic_streams.request.custom_record)
(output "Anthropic stream state properties passed\n")
