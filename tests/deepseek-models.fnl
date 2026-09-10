;; DeepSeek V4.1 facts, wire quirks, and peak pricing contracts. The model list
;; stays dynamic; published capability, price, and reasoning data merges onto
;; discovered rows.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local stock (require :tests.stock))
(local deepseek (require :misa.providers.deepseek))
(local openai (require :misa.protocols.openai))
(local models (require :misa.models))
(local costs (require :misa.costs))
(local preview (require :misa.models.preview))

(each [_ name (ipairs [:misa.json
                       :misa.protocols.openai
                       :misa.providers.deepseek
                       :misa.models
                       :misa.models.preview])]
  (app.include (. stock name)))
(app.install)

(local wiring (require :misa.standard.providers.deepseek))

(fn find [items id]
  (accumulate [found nil _ item (ipairs items) &until found]
    (when (= item.id id) item)))

;; The provider declares facts only for identifiers whose published data it
;; carries; a new identifier keeps its discovered row.
(local enrichment (deepseek.enrichment))
(local flash (find enrichment :deepseek/deepseek-flash))
(local pro (find enrichment :deepseek/deepseek-v4-pro))
(assert flash "DeepSeek V4.1 Flash was not declared")
(assert (= flash.context_window 1000000) "Flash lost its context window")
(assert (= flash.pricing.input 0.15) "Flash lost its off-peak input price")
(assert (= flash.pricing.cache_read 0.003) "Flash lost its cache price")
(assert (= pro.pricing.input 0.66) "Pro lost its off-peak input price")
(assert (= (find enrichment :deepseek/deepseek-pro) nil)
        "undeclared DeepSeek identifier invented facts")
(assert (= (. (find enrichment :deepseek/deepseek-v4-flash) :pricing :input)
           0.15)
        "the retired V4 Flash name lost the V4.1 Flash price")

;; Reasoning choices follow the published ladder, including the documented
;; `none` that disables thinking mode. DeepSeek advertises one ladder for the V4
;; family; models.dev still lists V4 Pro as high/max only.
(let [options (. flash.api :request_options :reasoning_effort)
      pro-options (. pro.api :request_options :reasoning_effort)]
  (assert (= (. flash.api :request_options_serializer) :openai.chat.deepseek))
  (assert (= options.default :high))
  (assert (= (length options.choices) 4))
  (assert (= (. options.choices 1) :none))
  (assert (= (. options.choices 4) :max))
  (assert (= (length pro-options.choices) 4))
  (assert (= (. pro-options.choices 2) :low)))

;; DeepSeek rejects forced tool choice while thinking, and names its output
;; limit `max_tokens`.
(assert (misa.request-options.serializable? :openai.chat.deepseek :max_tokens))
(assert (misa.request-options.serializable? :openai.chat.deepseek
                                            :reasoning_effort))
(assert (not (misa.request-options.serializable? :openai.chat.deepseek
                                                 :tool_choice)))
(assert (not (misa.request-options.serializable? :openai.chat.deepseek
                                                 :parallel_tool_calls)))
(assert (= (. (deepseek.serializer.serialize :max_tokens 4096) :max_tokens)
           4096))
(assert (= (. (deepseek.serializer.serialize :reasoning_effort :max)
              :reasoning_effort)
           :max))

;; Reasoning turns emit no stream bytes while thinking, so the transport widens
;; its first-byte and idle budgets without changing unrelated timeouts.
(let [config (deepseek.settings {})
      tuned (deepseek.settings {:timeouts {:idle_ms 1000 :overall_ms 2000}})]
  (assert (= config.max_tokens_field :max_tokens))
  (assert (= config.reasoning_content_field :reasoning_content))
  (assert (= config.timeouts.first_byte_ms 300000))
  (assert (= config.timeouts.idle_ms 300000))
  (assert (= tuned.timeouts.idle_ms 1000))
  (assert (= tuned.timeouts.first_byte_ms 300000))
  (assert (= tuned.timeouts.overall_ms 2000))
  (assert (= config.models_url "https://api.deepseek.com/models"))
  (assert (= config.url "https://api.deepseek.com/chat/completions"))
  (let [request (openai.request (deepseek.settings {:max_tokens 2048})
                                :openai.chat.deepseek
                                {:id :request
                                 :messages []
                                 :model "deepseek-flash"
                                 :tools []})]
    (assert (= request.json.max_tokens 2048)
            "the configured output limit used OpenAI's field name")
    (assert (= request.json.max_completion_tokens nil))))

;; A request carries DeepSeek's field names and no tool choice.
(let [effect (. wiring.effects :provider.deepseek)
      request (effect {:id :request
                       :messages [{:content [{:text "hi" :type :text}]
                                   :role :user}]
                       :model "deepseek-flash"
                       :request_options {:max_tokens 4096
                                         :reasoning_effort :high}
                       :tools []})]
  (assert (= request.type :http/request))
  (assert (= request.response_format :sse_json_stream))
  (assert (= (. request.credential :id) :deepseek))
  (assert (= request.json.max_tokens 4096))
  (assert (= request.json.max_completion_tokens nil))
  (assert (= request.json.model "deepseek-flash"))
  (assert (= request.json.reasoning_effort :high))
  (assert (= request.json.tool_choice nil))
  (assert (= request.json.stream true))
  (assert (= (. request.json.messages 1 :role) :user)))

;; Captured thinking returns on `reasoning_content`, which DeepSeek requires for
;; tool-call turns and ignores elsewhere.
(let [messages [{:content [{:type :thinking :text "weighing"}
                           {:type :tool_call
                            :arguments {:command "ls"}
                            :id :call-1
                            :name :shell}]
                 :role :assistant}
                {:content [{:type :thinking :text "idle"}]
                 :role :assistant}]
      replayed (openai.messages messages
                                {:reasoning_content_field :reasoning_content})
      plain (openai.messages messages nil)]
  (assert (= (. replayed 1 :reasoning_content) "weighing"))
  (assert (= (. replayed 2 :reasoning_content) "idle"))
  (assert (= (. replayed 1 :tool_calls 1 :function :name) "shell"))
  (assert (= (. plain 1 :reasoning_content) nil))
  (assert (= (. plain 2 :reasoning_content) nil)))

;; DeepSeek's model list reports identifiers only, so declared facts merge onto
;; the discovered catalogue instead of replacing it.
(let [db {:models {:available {}
                   :catalogue [{:id :deepseek/deepseek-flash
                                :model :deepseek-flash
                                :provider :deepseek}
                               {:id :deepseek/deepseek-pro
                                :model :deepseek-pro
                                :provider :deepseek}]
                   :entries []
                   :roles {}
                   :selected :deepseek/deepseek-flash}}
      handler (. (. wiring.events :provider.deepseek/catalogue) :handler)
      event (. (handler db {:provider :deepseek}) :fx 1 :event)
      patch (. (models.on-models-update {} db event) :patch)
      patched (. (misa.patch db patch) :models :catalogue)
      flash (find patched :deepseek/deepseek-flash)
      unknown (find patched :deepseek/deepseek-pro)]
  (assert (= event.type :models/update))
  (assert (= event.provider :deepseek))
  (assert (= (length event.models) (length (deepseek.enrichment))))
  (assert flash "the discovered row disappeared")
  (assert (= flash.context_window 1000000)
          "a discovered row kept no context window")
  (assert (= flash.pricing.output 0.6) "a discovered row kept no price")
  (assert (= (. flash.api :request_options :reasoning_effort :default) :high))
  (assert (= unknown.context_window nil)
          "an undeclared row gained a context window")
  (assert (= unknown.pricing nil) "an undeclared row gained prices"))

;; DeepSeek doubles its prices during UTC peak hours on weekdays.
(let [pricing {:cache_read 0.003
               :input 0.15
               :output 0.6
               :peak {:multiplier 2
                      :weekdays [2 3 4 5 6]
                      :windows [{:end_hour 4 :start_hour 1}
                                {:end_hour 10 :start_hour 6}]}}
      at (fn [seconds] (* seconds 1000))]
  (assert (= (costs.peak-multiplier pricing (at 1789351200)) 2)
          "a weekday peak hour was billed off-peak")
  (assert (= (costs.peak-multiplier pricing (at 1789369200)) 2)
          "the second peak window was billed off-peak")
  (assert (= (costs.peak-multiplier pricing (at 1789362000)) 1)
          "an off-peak hour was billed at peak rates")
  (assert (= (costs.peak-multiplier pricing (at 1789783200)) 1)
          "a weekend hour was billed at peak rates")
  (assert (= (costs.peak-multiplier pricing nil) 1)
          "prices without a request instant were billed at peak rates")
  (let [peak (costs.effective pricing (at 1789351200))]
    (assert (= peak.input 0.3))
    (assert (= peak.output 1.2))
    (assert (= peak.cache_read 0.006))
    (assert (= (. peak :peak :multiplier) 2)
            "peak windows were dropped from prices")
    (assert (= (. (costs.estimate peak {:input_includes_cache false
                                        :input_tokens 1000000
                                        :output_tokens 1000000})
                  :usd)
               1.5)
            "peak rates did not reach cost estimation"))
  (assert (= (. (costs.effective pricing (at 1789362000)) :input) 0.15)
          "an off-peak request was charged peak rates"))

;; Model previews state the peak windows instead of hiding them.
(let [lines (preview.model-preview {:context_window 1000000
                                    :title "deepseek/deepseek-flash"
                                    :cost {:currency :USD
                                           :peak {:multiplier 2
                                                  :weekdays [2 3 4 5 6]
                                                  :windows [{:end_hour 4
                                                             :start_hour 1}
                                                            {:end_hour 10
                                                             :start_hour 6}]}
                                           :pricing {:cache_read 0.003
                                                     :input 0.15
                                                     :output 0.6}
                                           :token_unit 1000000
                                           :unavailable false}}
                                   {})]
  (assert (= (length lines) 6))
  (assert (= (. lines 5 :spans 1 :text)
             "Peak 2× at 01:00–04:00, 06:00–10:00 UTC on Mon–Fri")))

(output "deepseek model contracts passed\n")
