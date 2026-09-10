(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local command (require :misa.providers.command))
(local fake (require :misa.providers.fake))
(local auth (require :misa.providers.auth))
(local agent (require :misa.agent))
(local stream (require :misa.agent.stream))
(local router (require :misa.providers.openrouter))
(local openai-options (require :misa.providers.openai-options))
(local kimi (require :misa.providers.kimi))

;; Policies can be called without constructing or installing an application.
(let [argv [:adapter :--prompt]
      request (command.request argv
                               {:id :request
                                :messages [{:role :user
                                            :content [{:type :text :text "one"}
                                                      {:type :image}
                                                      {:type :text :text "two"}]}]})]
  (assert (= (length argv) 2) "request construction mutated configuration")
  (assert (= (. request.argv 3) "one\ntwo"))
  (assert (= request.type :provider/process))
  (assert (= request.completion :provider/command-complete)))

(let [success (command.complete nil {:id :request :ok true :stdout "answer\n"})
      failure (command.complete nil
                                {:id :request :ok false :stderr "" :status 7})]
  (assert (= (. success.fx 2 :event :delta :text) "answer\n"))
  (assert (= (. success.fx 3 :event :type) :agent/stream-end))
  (assert (= (. failure.fx 2 :event :message) "command exited 7")))

(let [responses ["first" "second"]
      previous {:providers {:unrelated {:value 1}}}
      first (fake.respond responses previous {:id :one})
      second (fake.respond responses (misa.patch previous first.patch)
                           {:id :two})]
  (assert (= previous.providers.fake nil)
          "response reduction mutated prior state")
  (assert (= (. first.fx 2 :event :delta :text) "first"))
  (assert (= (. second.fx 2 :event :delta :text) "second")))

(let [empty (auth.startup [] {})
      pending (auth.startup [{:id :test
                              :model_provider :model
                              :strategy :api_key}]
                            {})]
  (assert (= (. (misa.patch {} empty.patch) :auth_startup :ready) true))
  (assert (= (. empty.fx 1 :event :type) :auth/startup-ready))
  (assert (= (. pending.fx 1 :completion) :auth/provider-status))
  (assert (= (. pending.fx 1 :provider) :test)))

(let [previous {:agent {:status :working :active_request_id :request}}
      cancelled (agent.cancel previous)]
  (assert (= previous.agent.cancel_requested nil))
  (assert (= cancelled.patch.agent.status :cancelling))
  (assert (= (. cancelled.fx 2 :type) :operation/cancel))
  (assert (= (agent.cancel (misa.patch previous cancelled.patch)) nil)
          "repeated cancellation emitted duplicate effects"))

(let [pricing (router.model-pricing {:pricing {:prompt "0.000002"
                                               :completion "0.000004"
                                               :input_cache_read "0.000001"
                                               :request "0.01"}})]
  (assert (= pricing.input 2))
  (assert (= pricing.output 4))
  (assert (= pricing.cache_read 1))
  (assert (= pricing.request 0.01)))

(let [serializer (openai-options.compose [router.routing router.reasoning
                                           openai-options.standard])
      routing {:only [:anthropic :google]
               :require_parameters true
               :data_collection :deny}
      configured (router.settings {:routing routing
                                   :request_options {:temperature 0.2}})
      request (misa.patch (serializer.serialize :temperature
                                                configured.request_options.temperature)
                          (serializer.serialize :provider
                                                configured.request_options.provider))]
  (assert (= request.temperature 0.2))
  (assert (= (. request.provider.only 1) :anthropic))
  (assert (= (. request.provider.only 2) :google))
  (assert (= request.provider.require_parameters true))
  (assert (= request.provider.data_collection :deny))
  (assert (= (. (serializer.serialize :reasoning_effort :high) :reasoning :effort)
             :high)))

(let [plain (router.settings {})]
  (assert (= (next plain.request_options) nil)
          "OpenRouter settings must work without a routing policy"))

(let [windows (kimi.usage-windows {:usage {:limit 100 :remaining 25}})]
  (assert (= (. windows 1 :used) 75))
  (assert (= (. windows 1 :remaining) 25)))

(let [plain {:type :dispatch
             :event {:type :agent/stream-delta
                     :id :request
                     :delta {:type :text :text "a"}}}
      annotated (misa.patch plain {:event {:custom true}})
      folded (stream.effects [plain annotated plain])]
  (assert (= (length folded) 3) "custom event fields lost a batching boundary")
  (assert (= (. folded 2) annotated)))

(output "provider policy contracts passed\n")
