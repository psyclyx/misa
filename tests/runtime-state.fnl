(local fennel (require :fennel))
(local dofile fennel.dofile)

;; Real framework snapshots; native effects are observed without network access.

(local output io.write)

(local context {:argv {} :config {}})

(dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local declarations (require :misa.definitions))
(app.include (fennel.dofile :extensions/misa/agent/stream.fnl) {})

(each [_ name (ipairs [:misa.json
                       :misa.protocols.openai
                       :misa.protocols.anthropic
                       :misa.providers.openai-codex
                       :misa.agent
                       :misa.editor.queue])]
  (app.include (require name) context))

(app.define ((. (require :misa.protocols.openai) :configure) {:id :fixture-chat
                                             :models {}
                                             :url "https://fixture.invalid"}))

(app.define ((. (require :misa.protocols.anthropic) :configure) {:id :fixture-anthropic
                                                :models {}
                                                :url "https://fixture.invalid"}))

(app.define (declarations :runtime-state-4 [(let [definition {:description "Fixture tool"
                                    :effect :capture/tool
                                    :input_schema {:properties {:value {:type :string}}
                                                   :required [:value]
                                                   :type :object}
                                    :name :fixture}] {:catalog :tools :id (. definition :name) :value definition})]))

(var (snapshot native observed) (values nil {} {}))

(app.define (declarations :runtime-state-5 [{:catalog :events  :value {:event :test/read :handler (fn [db] (set snapshot db) nil)}}]))

(app.define (declarations :runtime-state-6 [{:catalog :events  :value {:event :test/patch :handler (fn [_]
                                       {:patch {:patch_probe {:value :updated}}})}}]))

(var subscription-evaluations 0)
(app.define (declarations :runtime-state-7 [(let [definition {:id :test/doubled
                                    :inputs (fn [_] [[:db/path :value]])
                                    :compute (fn [inputs _]
                                               (set subscription-evaluations
                                                    (+ subscription-evaluations 1))
                                               (* (. inputs 1) 2))}] {:catalog :subscriptions :id (. definition :id) :value definition})]))

(app.define (declarations :runtime-state-8 [{:catalog :events  :value {:event :app/start :handler (fn [db]
                                       {:patch {:models
                                            {:entries [{:id :openai-codex/gpt-5.4
                                                        :model :gpt-5.4
                                                        :provider :openai-codex}]
                                             :selected :openai-codex/gpt-5.4}}})}}]))

(app.define (declarations :runtime-state-9 [{:catalog :events  :value {:event :editor/restore :handler (fn [_ event]
                                       (table.insert observed event) nil)}}]))

(app.install context)

(fn dispatch [event]
  (let [pending [event]]
    (var at 1)
    (while (. pending at)
      (local effects
             (misa._dispatch (. pending at)
                             {:columns 80 :interactive false :lines 24}
                             {:monotonic_ms 0 :wall_ms 0}))
      (misa._commit)
      (set at (+ at 1))
      (each [_ effect (ipairs effects)]
        (if (= effect.type :dispatch)
            (tset pending (+ (length pending) 1) effect.event)
            (tset native (+ (length native) 1) effect)))
      (assert (< at 1000) "dispatch loop"))
    (local effects
           (misa._dispatch {:type :test/read}
                           {:columns 80 :interactive false :lines 24}
                           {:monotonic_ms 0 :wall_ms 0}))
    (misa._commit)
    (assert (= (length effects) 0))
    nil))

(fn last [kind]
  (for [index (length native) 1 (- 1)]
    (when (= (. native index :type) kind)
      (let [___antifnl_rtn_1___ (. native index)]
        (lua "return ___antifnl_rtn_1___"))))
  nil)

(fn codex [records]
  (dispatch {:id snapshot.agent.active_request_id
             :phase :records
             : records
             :type :provider/openai-codex-complete})
  nil)

(local image {:source {:data :aW1hZ2U= :media_type :image/png :type :base64}
              :type :image})

(dispatch {:type :app/start})

(dispatch {:attachments [image] :prompt :hello :type :queue/submit})

(var request (last :http/request))

(assert (and (= (. request.json.tools 1 :name) :fixture)
             (= (. request.json.tools 1 :parameters :properties :value :type)
                :string)) "tools absent from real request")

(assert (= (. request.json.input 1 :content 2 :type) :input_image)
        "Codex image attachment missing")

(dispatch {:prompt :next :type :queue/submit})

(dispatch {:attachments [image] :prompt :then :type :queue/submit})

(assert (and (= snapshot.queue.pending "next\nthen")
             (= (length snapshot.queue.attachments) 1))
        "queue did not coalesce")

(dispatch {:type :queue/take})

(assert (and (= snapshot.queue.pending "")
             (= (length snapshot.queue.attachments) 0)))

(var restored nil)

(each [_ event (ipairs observed)]
  (when (= event.type :editor/restore) (set restored event)))

(assert (and (= restored.text "next\nthen") (= (length restored.attachments) 1))
        "queue edit lost text or image")

(dispatch {:prompt :later :type :queue/submit})

(dispatch {:id :agent-1 :phase :start :type :provider/openai-codex-complete})

(codex [{:delta "Considering the request."
         :output_index 0
         :type :response.reasoning_summary_text.delta}])

(codex [{:item {:encrypted_content :opaque
                :id :reason-1
                :summary [{:text "Considering the request."
                           :type :summary_text}]
                :type :reasoning}
         :output_index 0
         :type :response.output_item.done}])

(codex [{:item {:arguments ""
                :call_id :call-1
                :name :fixture
                :type :function_call}
         :output_index 1
         :type :response.output_item.added}])

(assert (= (. snapshot.agent.stream.blocks 2 :name) :fixture)
        "tool pending row not emitted at start")

(codex [{:delta "{\"value\":\"ok\"}"
         :output_index 1
         :type :response.function_call_arguments.delta}])

(assert (= (table.concat (. snapshot.agent.stream.blocks 2
                            :arguments_json_chunks))
           "{\"value\":\"ok\"}") "tool arguments not streamed")

(codex [{:item {:arguments "{\"value\":\"ok\"}"
                :call_id :call-1
                :name :fixture
                :type :function_call}
         :output_index 1
         :type :response.output_item.done}
        {:response {:usage {:cost 0.012 :input_tokens 10 :output_tokens 4}}
         :type :response.completed}])

(dispatch {:id :agent-1
           :ok true
           :phase :end
           :type :provider/openai-codex-complete})

(assert (and (= snapshot.agent.status :tools)
             (= (. (last :capture/tool) :arguments :value) :ok))
        "streamed call did not execute")

(assert (= (. snapshot.agent.messages 2 :content 1 :text)
           "Considering the request.") "reasoning duplicated/lost")

(assert (and (= snapshot.agent.last_usage.cost_usd 0.012)
             (= snapshot.agent.last_usage.input_includes_cache true))
        "usage metadata lost")

(dispatch {:text :done :tool_call_id :call-1 :type :tool/result})

(set request (last :http/request))

(assert (and (= (. request.json.input 2 :type) :reasoning)
             (= (. request.json.input 2 :encrypted_content) :opaque))
        "reasoning state missing from tool continuation")

(assert (= snapshot.queue.pending :later)
        "pending prompt interrupted tool loop")

(dispatch {:id :agent-2 :phase :start :type :provider/openai-codex-complete})

(codex [{:delta "Partial answer" :type :response.output_text.delta}])

(dispatch {:attachments [image] :prompt "change course" :type :queue/steer})

(assert (and (= (. (last :operation/cancel) :id) :agent-2)
             (= snapshot.agent.status :cancelling)))

(dispatch {:id :agent-2
           :message :cancelled
           :ok false
           :phase :end
           :type :provider/openai-codex-complete})

(assert (and (= snapshot.agent.status :working)
             (= snapshot.agent.request_seq 3)) "steer did not resume")

(assert (= (. snapshot.agent.messages (length snapshot.agent.messages) :content
              1 :text) "later\nchange course")
        "steer lost pending prompt")

(assert (= (. snapshot.agent.messages (- (length snapshot.agent.messages) 1)
              :content 1 :text) "Partial answer")
        "interrupt lost visible assistant context")

(assert (= (. snapshot.agent.messages (length snapshot.agent.messages) :content
              2 :type) :image) "steer lost image")

(dispatch {:type :agent/reset})

(assert (and (= snapshot.queue.pending "") (= snapshot.agent.status :ready)))

(dispatch {:attachments [image] :prompt "" :type :queue/submit})

(assert (= (. snapshot.agent.messages 1 :content 1 :type) :image)
        "image-only submit failed")

;; Shared protocol serializers preserve attachments and signed thinking.

(local chat
       (misa.protocols.openai-messages [{:content [image]
                                                   :role :user}]))

(assert (= (. chat 1 :content 1 :image_url :url)
           "data:image/png;base64,aW1hZ2U="))

(local anthropic
       (misa.protocols.anthropic-messages [{:content [{:text :visible
                                                                 :type :thinking}
                                                                {:arguments {}
                                                                 :id :c
                                                                 :name :fixture
                                                                 :type :tool_call}]
                                                      :provider_state [{:provider :anthropic
                                                                        :value {:signature :signed
                                                                                :thinking :visible
                                                                                :type :thinking}}]
                                                      :role :assistant}
                                                     {:content [image]
                                                      :role :user}]))

(assert (and (= (. anthropic 1 :content 1 :signature) :signed)
             (= (. anthropic 1 :content 2 :type) :tool_use))
        "signed thinking not replayed before tool call")

(assert (= (. anthropic 2 :content 1 :type) :image))

(local switched
       (misa.protocols.anthropic-messages [{:content [{:text :visible
                                                                 :type :text}]
                                                      :provider_state [{:provider :anthropic
                                                                        :value {:signature :signed
                                                                                :thinking :private
                                                                                :type :thinking}}]
                                                      :role :assistant}]
                                                    :kimi))

(assert (and (= (length (. switched 1 :content)) 1)
             (= (. switched 1 :content 1 :type) :text))
        "provider switch replayed a foreign reasoning signature")

;; Immutable patches preserve identity for unrelated branches and are no-ops
;; when they do not change the value.
(local patch-state {:left {:value 1} :right {:value 2}})
(local patch-noop (misa.patch patch-state {:left {:value 1}}))
(local patch-next (misa.patch patch-state {:left {:value 3}}))
(assert (= patch-noop patch-state) "no-op patch rebuilt state")
(assert (and (not= patch-next patch-state)
             (= patch-next.right patch-state.right)
             (= patch-next.left.value 3))
        "patch did not preserve structural sharing")
(local patch-replaced (misa.patch patch-state
                                  {:left (misa.replace {:only :this})}))
(local patch-deleted (misa.patch patch-state {:right misa.delete}))
(assert (and (= patch-replaced.left.only :this)
             (= patch-replaced.left.value nil)
             (= patch-deleted.right nil))
        "patch replace/delete semantics failed")
(local nested-delete (misa.patch {:new {:keep true :remove true}}
                                 {:new {:remove misa.delete}}))
(assert (and nested-delete.new.keep (= nested-delete.new.remove nil))
        "nested delete leaked patch control data")

(dispatch {:type :test/patch})
(assert (= snapshot.patch_probe.value :updated)
        "event patch was not committed")

(local subscription-state {:value 21})
(assert (= (misa.sub subscription-state [:test/doubled]) 42))
(assert (= (misa.sub subscription-state [:test/doubled]) 42))
(assert (= subscription-evaluations 1)
        "subscription did not reuse unchanged input values")

(output "runtime state regressions passed\n")

nil
