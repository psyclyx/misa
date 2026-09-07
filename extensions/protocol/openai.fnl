;; OpenAI-compatible streaming projections operate on records, not UI state.
(fn text-delta [kind value]
  (if (and (= (type value) :string) (not= value ""))
      [{:type kind :text value}] []))

(local delta-projections
       [{:id :text :project (fn [delta] (text-delta :text delta.content))}
        {:id :thinking :project (fn [delta] (text-delta :thinking (or delta.reasoning_content delta.reasoning)))}
        {:id :tool_calls
         :project (fn [delta]
                    (local result [])
                    (each [_ call (ipairs (or delta.tool_calls []))]
                      (local function-data (if (= (type (. call :function)) :table) (. call :function) {}))
                      (table.insert result {:type :tool_call :arguments_json_delta function-data.arguments
                                            :id call.id :index (or call.index 0) :name function-data.name}))
                    result)}])
(local delta-ids {:text true :thinking true :tool_calls true})

(fn stream-update [id terminal fx]
  {:patch {:providers {:openai_streams {id (misa.replace terminal)}}} : fx})

(fn stream [db event]
  (local previous (and db.providers db.providers.openai_streams
                       (. db.providers.openai_streams event.id)))
  (if (= event.phase :start)
      (stream-update event.id false [{:type :dispatch :event {:id event.id :type :agent/stream-start}}])
      (= event.phase :end)
      (let [body (when (and (= (type event.body) :string) (not= event.body "")) event.body)
            http (when (and (= (type event.status) :number) (>= event.status 400))
                   (.. "HTTP " event.status (if body (.. ": " body) "")))
            completed (and event.ok (= previous true))
            result (if completed {:id event.id :type :agent/stream-end}
                       {:id event.id :type :agent/stream-error
                        :message (or http body event.message
                                     (when (not previous) "OpenAI stream ended without [DONE]")
                                     "OpenAI request failed")})]
        (stream-update event.id nil [{:type :dispatch :event result}]))
      (= previous true) nil
      (let [fx []]
        (each [_ record (ipairs (or event.records []))]
          (local choice (and (= (type record.choices) :table) (. record.choices 1)))
          (local delta (and choice choice.delta))
          (when (= (type delta) :table)
            (each [_ projection (ipairs delta-projections)]
              (each [_ value (ipairs (or (projection.project delta record) []))]
                (table.insert fx {:type :dispatch :event {:id event.id :type :agent/stream-delta :delta value}}))))
          (local raw-usage (when (= (type record.usage) :table) record.usage))
          (when (or raw-usage (and choice choice.finish_reason))
            (local usage (or raw-usage {}))
            (local details (if (= (type usage.prompt_tokens_details) :table) usage.prompt_tokens_details {}))
            (table.insert fx {:type :dispatch
                              :event {:id event.id :type :agent/stream-usage
                                      :stop_reason (and choice choice.finish_reason)
                                      :usage {:cache_read_tokens (or details.cached_tokens 0)
                                              :cache_write_tokens 0 :cost_usd usage.cost
                                              :input_includes_cache true :input_tokens (or usage.prompt_tokens 0)
                                              :output_tokens (or usage.completion_tokens 0)}}})))
        (when (= event.terminal true) (table.insert fx {:id event.id :type :operation/finish}))
        (stream-update event.id (= event.terminal true) fx))))

{:setup (fn [context]
          (local setup-fx [])
          ;; OpenAI-compatible Chat Completions protocol adapter.

          (fn text [content]
            (if (= (type content) :string) content
                (let [parts {}]
                  (each [_ block (ipairs (or content {}))]
                    (when (= block.type :text)
                      (tset parts (+ (length parts) 1) block.text)))
                  (table.concat parts ""))))

          (fn messages [___values___]
            (let [result {}]
              (each [_ message (ipairs ___values___)]
                (if (= message.role :user)
                    (do
                      (var (parts has-image) (values {} false))
                      (each [_ block (ipairs (or message.content {}))]
                        (if (= block.type :text)
                            (tset parts (+ (length parts) 1)
                                  {:text block.text :type :text})
                            (= block.type :image)
                            (do
                              (set has-image true)
                              (tset parts (+ (length parts) 1)
                                    {:image_url {:url (.. "data:"
                                                          block.source.media_type
                                                          ";base64,"
                                                          block.source.data)}
                                     :type :image_url}))))
                      (tset result (+ (length result) 1)
                            {:content (or (and has-image parts)
                                          (text message.content))
                             :role :user}))
                    (= message.role :assistant)
                    (let [item {:content (text message.content)
                                :role :assistant}
                          calls {}]
                      (each [_ block (ipairs (or message.content {}))]
                        (when (= block.type :tool_call)
                          (tset calls (+ (length calls) 1)
                                {:function {:arguments ((. (assert misa.json
                                                                   "protocol.openai requires json for tool history")
                                                           :encode) block.arguments)
                                            :name block.name}
                                 :id block.id
                                 :type :function})))
                      (when (> (length calls) 0) (set item.tool_calls calls))
                      (tset result (+ (length result) 1) item))
                    (= message.role :tool)
                    (tset result (+ (length result) 1)
                          {:content (text message.content)
                           :role :tool
                           :tool_call_id message.tool_call_id})))
              result))

          (fn tools [___values___]
            (let [result {}]
              (each [_ tool (ipairs ___values___)]
                (tset result (+ (length result) 1)
                      {:function {:description tool.description
                                  :name tool.name
                                  :parameters tool.input_schema}
                       :type :function}))
              result))

          (table.insert setup-fx
                        {:type :register/service
                         :name :protocols.serialize_openai_messages
                         :value messages})
          (table.insert setup-fx
                        {:type :register/service
                         :name :protocols.openai
                         :value (fn [spec]
                                  (local setup-fx [])
                                  (assert (and (and (= (type spec.id) :string)
                                                    (= (type spec.url) :string))
                                               (= (type spec.models) :table)))
                                  (local serializer-id
                                         (.. :openai.chat. spec.id))
                                  (table.insert setup-fx
                                                {:type :register/request-options-serializer
                                                 :id serializer-id
                                                 :serializer {:accepts (fn [name]
                                                                         (= name
                                                                            :reasoning_effort))
                                                              :serialize (fn [body
                                                                              name
                                                                              value]
                                                                           (if (not= name
                                                                                     :reasoning_effort)
                                                                               false
                                                                               (do
                                                                                 (if spec.reasoning_effort
                                                                                     (spec.reasoning_effort body
                                                                                                            value)
                                                                                     (set body.reasoning_effort
                                                                                          value))
                                                                                 true)))}})

                                  (fn model-api [source]
                                    (if (and (= (type source) :table)
                                             (= (type source.request_options) :table))
                                        (misa.patch source {:request_options_serializer serializer-id}) source))

                                  (each [_ model (ipairs spec.models)]
                                    (table.insert setup-fx
                                                  {:type :register/model
                                                   :value {:api (model-api model.api)
                                                           :context_window model.context_window
                                                           :id model.id
                                                           :label (or model.label
                                                                      model.id)
                                                           :model model.model
                                                           :pricing model.pricing
                                                           :provider spec.id}}))
                                  (when spec.models_url
                                    (fn discover []
                                      (let [credential (when (not= spec.models_credential false)
                                                           {:header :authorization
                                                            :id spec.credential
                                                            :prefix "Bearer "})]
                                        {:fx [{:completion (.. :provider/
                                                               spec.id :-models)
                                               : credential
                                               :headers (or spec.model_headers
                                                            {})
                                               :id (.. :models- spec.id)
                                               :method :GET
                                               :response_format :json
                                               :timeouts spec.timeouts
                                               :type :http/request
                                               :url spec.models_url}]}))

                                    (table.insert setup-fx
                                                  {:type :register/event
                                                   :name :models/discover
                                                   :handler (fn [_ event]
                                                              (if (and event.provider
                                                                       (not= event.provider
                                                                             spec.id))
                                                                  nil
                                                                  (discover)))})
                                    (table.insert setup-fx
                                                  {:type :register/event
                                                   :name (.. :provider/ spec.id
                                                             :-models)
                                                   :handler (fn [_ event]
                                                              (if (or (or (not event.ok)
                                                                          (not= (type event.data)
                                                                                :table))
                                                                      (not= (type event.data.data)
                                                                            :table))
                                                                  {:fx [{:event {:provider spec.id
                                                                                 :type :models/discovery-complete}
                                                                         :type :dispatch}]}
                                                                  (let [discovered {}]
                                                                    (each [_ item (ipairs event.data.data)]
                                                                      (when (and (and (= (type item)
                                                                                         :table)
                                                                                      (= (type item.id)
                                                                                         :string))
                                                                                 (or (not spec.model_filter)
                                                                                     (spec.model_filter item)))
                                                                        (tset discovered
                                                                              (+ (length discovered)
                                                                                 1)
                                                                              {:api (model-api (or item.api
                                                                                                   (or (and (= (type item.request_options)
                                                                                                               :table)
                                                                                                            {:request_options item.request_options})
                                                                                                       nil)))
                                                                               :context_window (or item.context_length
                                                                                                   item.context_window)
                                                                               :id (.. spec.id
                                                                                       "/"
                                                                                       item.id)
                                                                               :label (or item.name
                                                                                          item.id)
                                                                               :model item.id
                                                                               :pricing (or (and spec.model_pricing
                                                                                                 (spec.model_pricing item))
                                                                                            nil)})))
                                                                    (table.sort discovered
                                                                                (fn [left
                                                                                     right]
                                                                                  (< left.id
                                                                                     right.id)))
                                                                    {:fx [{:event {:authoritative (= spec.catalogue_authoritative
                                                                                                     true)
                                                                                   :models discovered
                                                                                   :provider spec.id
                                                                                   :type :models/replace-provider}
                                                                           :type :dispatch}
                                                                          {:event {:provider spec.id
                                                                                   :type :models/discovery-complete}
                                                                           :type :dispatch}]})))}))
                                  (table.insert setup-fx
                                                {:type :register/fx
                                                 :name (.. :provider. spec.id)
                                                 :handler (fn [effect]
                                                            (let [converted (messages effect.messages)]
                                                              (when effect.system_prompt
                                                                (table.insert converted
                                                                              1
                                                                              {:content effect.system_prompt
                                                                               :role :system}))
                                                              (local body
                                                                     {:messages converted
                                                                      :model effect.model
                                                                      :stream true
                                                                      :stream_options {:include_usage true}})
                                                              (local definitions
                                                                     (tools effect.tools))
                                                              (when (> (length definitions)
                                                                       0)
                                                                (set body.tools
                                                                     definitions))
                                                              (when spec.max_tokens
                                                                (set body.max_completion_tokens
                                                                     spec.max_tokens))
                                                              (misa.serialize_request_options serializer-id
                                                                                              (or effect.request_options
                                                                                                  {})
                                                                                              body)
                                                              (local headers
                                                                     [{:name :content-type
                                                                       :value :application/json}])
                                                              (each [_ header (ipairs (or spec.headers
                                                                                          {}))]
                                                                (tset headers
                                                                      (+ (length headers)
                                                                         1)
                                                                      header))
                                                              {:completion (.. :provider/
                                                                               spec.id
                                                                               :-complete)
                                                               :credential {:header :authorization
                                                                            :id spec.credential
                                                                            :prefix "Bearer "}
                                                               : headers
                                                               :id effect.id
                                                               :json body
                                                               :method :POST
                                                               :response_format :sse_json_stream
                                                               :timeouts spec.timeouts
                                                               :type :http/request
                                                               :url spec.url}))})
                                  (table.insert setup-fx
                                                {:type :register/event
                                                 :name (.. :provider/ spec.id :-complete)
                                                 :handler stream})
                                  {:fx setup-fx})})
          (table.insert setup-fx
                        {:type :register/setup-effect :name :register/openai-delta
                         :handler (fn [effect]
                                    (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                 (= (type effect.value) :function) (not (. delta-ids effect.id)))
                                            "invalid or duplicate OpenAI delta projection")
                                    (tset delta-ids effect.id true)
                                    (table.insert delta-projections {:id effect.id :project effect.value}))})
          {:fx setup-fx})}
