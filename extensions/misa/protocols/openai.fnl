(local definitions (require :misa.definitions))

;; OpenAI-compatible streaming projections operate on records, not UI state.
(fn text-delta [kind value]
  (if (and (= (type value) :string) (not= value ""))
      [{:type kind :text value}]
      []))

(local delta-projections [{:id :text
                           :project (fn [delta]
                                      (text-delta :text delta.content))}
                          {:id :thinking
                           :project (fn [delta]
                                      (text-delta :thinking
                                                  (or delta.reasoning_content
                                                      delta.reasoning)))}
                          {:id :tool_calls
                           :project (fn [delta]
                                      (let [result []]
                                        (each [_ call (ipairs (or delta.tool_calls
                                                                  []))]
                                          (let [function-data (if (= (type (. call
                                                                              :function))
                                                                     :table)
                                                                  (. call
                                                                     :function)
                                                                  {})]
                                            (table.insert result
                                                          {:type :tool_call
                                                           :arguments_json_delta function-data.arguments
                                                           :id call.id
                                                           :index (or call.index
                                                                      0)
                                                           :name function-data.name})))
                                        result))}])

(fn stream-update [id terminal fx]
  {:patch {:providers {:openai_streams {id (misa.replace terminal)}}}
   :fx (misa.stream.effects fx)})

(fn stream [db event]
  (let [previous (and db.providers db.providers.openai_streams
                      (. db.providers.openai_streams event.id))]
    (if (= event.phase :start)
        (stream-update event.id false
                       [{:type :dispatch
                         :event {:id event.id :type :agent/stream-start}}])
        (= event.phase :end)
        (let [body (when (and (= (type event.body) :string)
                              (not= event.body ""))
                     event.body)
              http (when (and (= (type event.status) :number)
                              (>= event.status 400))
                     (.. "HTTP " event.status (if body (.. ": " body) "")))
              completed (and event.ok (= previous true))
              result (if completed {:id event.id :type :agent/stream-end}
                         {:id event.id
                          :type :agent/stream-error
                          :message (or http body event.message
                                       (when (not previous)
                                         "OpenAI stream ended without [DONE]")
                                       "OpenAI request failed")})]
          (stream-update event.id nil [{:type :dispatch :event result}]))
        (= previous true)
        nil
        (let [fx []]
          (each [_ record (ipairs (or event.records []))]
            (let [choice (and (= (type record.choices) :table)
                              (. record.choices 1))
                  delta (and choice choice.delta)]
              (when (= (type delta) :table)
                (each [_ projection (ipairs (misa.catalog-entries :openai-deltas))]
                  (each [_ value (ipairs (or (projection.value delta record) []))]
                    (table.insert fx
                                  {:type :dispatch
                                   :event {:id event.id
                                           :type :agent/stream-delta
                                           :delta value}}))))
              (let [raw-usage (when (= (type record.usage) :table) record.usage)]
                (when (or raw-usage (and choice choice.finish_reason))
                  (let [usage (or raw-usage {})
                        details (if (= (type usage.prompt_tokens_details)
                                       :table)
                                    usage.prompt_tokens_details
                                    {})]
                    (table.insert fx
                                  {:type :dispatch
                                   :event {:id event.id
                                           :type :agent/stream-usage
                                           :stop_reason (and choice
                                                             choice.finish_reason)
                                           :usage {:cache_read_tokens (or details.cached_tokens
                                                                          0)
                                                   :cache_write_tokens 0
                                                   :cost_usd usage.cost
                                                   :input_includes_cache true
                                                   :input_tokens (or usage.prompt_tokens
                                                                     0)
                                                   :output_tokens (or usage.completion_tokens
                                                                      0)}}}))))))
          (when (= event.terminal true)
            (table.insert fx {:id event.id :type :operation/finish}))
          (stream-update event.id (= event.terminal true) fx)))))

((fn [context]
   (let [declarations []]
     ;; OpenAI-compatible Chat Completions protocol adapter.

     (fn text [content]
       (if (= (type content) :string) content
           (let [parts {}]
             (each [_ block (ipairs (or content {}))]
               (when (= block.type :text)
                 (tset parts (+ (length parts) 1) block.text)))
             (table.concat parts ""))))

     (fn messages [items]
       "Serialize conversation messages for the Openai protocol."
       (let [result {}]
         (each [_ message (ipairs items)]
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
               (let [item {:content (text message.content) :role :assistant}
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

     (fn tools [items]
       (let [result {}]
         (each [_ tool (ipairs items)]
           (tset result (+ (length result) 1)
                 {:function {:description tool.description
                             :name tool.name
                             :parameters tool.input_schema}
                  :type :function}))
         result))

     (table.insert declarations
                   {:catalog :services
                    :id :protocols.openai-messages
                    :value messages})
     (let [configure (fn [spec]
                       "Describe this transport for a provider configuration."
                       (let [declarations []]
                         (assert (and (= (type spec.id) :string)
                                      (= (type spec.url) :string)
                                      (= (type spec.models) :table)))
                         (let [serializer-id (.. :openai.chat. spec.id)]
                           (table.insert declarations
                                         {:catalog :serializers
                                          :id serializer-id
                                          :value {:accepts (fn [name]
                                                             (= name
                                                                :reasoning_effort))
                                                  :serialize (fn [name value]
                                                               "Describe the request fields for a supported option."
                                                               (when (= name
                                                                        :reasoning_effort)
                                                                 (if spec.reasoning_effort
                                                                     (spec.reasoning_effort value)
                                                                     {:reasoning_effort value})))}})

                           (fn model-api [source]
                             (if (and (= (type source) :table)
                                      (= (type source.request_options) :table))
                                 (misa.patch source
                                             {:request_options_serializer serializer-id})
                                 source))

                           (each [_ model (ipairs spec.models)]
                             (table.insert declarations
                                           (let [definition {:api (model-api model.api)
                                                             :context_window model.context_window
                                                             :id model.id
                                                             :label (or model.label
                                                                        model.id)
                                                             :model model.model
                                                             :pricing model.pricing
                                                             :provider spec.id}]
                                             {:catalog :models
                                              :id (. definition :id)
                                              :value definition})))
                           (when spec.models_url
                             (fn discover []
                               (let [credential (when (not= spec.models_credential
                                                            false)
                                                  {:header :authorization
                                                   :id spec.credential
                                                   :prefix "Bearer "})]
                                 {:fx [{:completion (.. :provider/ spec.id
                                                        :-models)
                                        : credential
                                        :headers (or spec.model_headers {})
                                        :id (.. :models- spec.id)
                                        :method :GET
                                        :response_format :json
                                        :timeouts spec.timeouts
                                        :type :http/request
                                        :url spec.models_url}]}))

                             (table.insert declarations
                                           {:catalog :events
                                            :value {:event :models/discover
                                                    :handler (fn [_ event]
                                                               (if (and event.provider
                                                                        (not= event.provider
                                                                              spec.id))
                                                                   nil
                                                                   (discover)))}})
                             (table.insert declarations
                                           {:catalog :events
                                            :value {:event (.. :provider/
                                                               spec.id :-models)
                                                    :handler (fn [_ event]
                                                               (if (or (not event.ok)
                                                                       (not= (type event.data)
                                                                             :table)
                                                                       (not= (type event.data.data)
                                                                             :table))
                                                                   {:fx [{:event {:provider spec.id
                                                                                  :type :models/discovery-complete}
                                                                          :type :dispatch}]}
                                                                   (let [discovered {}]
                                                                     (each [_ item (ipairs event.data.data)]
                                                                       (when (and (= (type item)
                                                                                     :table)
                                                                                  (= (type item.id)
                                                                                     :string)
                                                                                  (or (not spec.model_filter)
                                                                                      (spec.model_filter item)))
                                                                         (tset discovered
                                                                               (+ (length discovered)
                                                                                  1)
                                                                               {:api (model-api (or item.api
                                                                                                    (and (= (type item.request_options)
                                                                                                            :table)
                                                                                                         {:request_options item.request_options})
                                                                                                    nil))
                                                                                :created (when (= (type item.created)
                                                                                                  :number)
                                                                                           item.created)
                                                                                :recommended (when spec.model_recommended
                                                                                               (spec.model_recommended item))
                                                                                :popularity_rank (when (= (type item.popularity_rank)
                                                                                                          :number)
                                                                                                   item.popularity_rank)
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
                                                                            :type :dispatch}]})))}}))
                           (table.insert declarations
                                         {:catalog :effects
                                          :id (.. :provider. spec.id)
                                          :value (fn [effect]
                                                   (let [converted (messages effect.messages)]
                                                     (when effect.system_prompt
                                                       (table.insert converted
                                                                     1
                                                                     {:content effect.system_prompt
                                                                      :role :system}))
                                                     (let [body {:messages converted
                                                                 :model effect.model
                                                                 :stream true
                                                                 :stream_options {:include_usage true}}
                                                           definitions (tools effect.tools)]
                                                       (when (> (length definitions)
                                                                0)
                                                         (set body.tools
                                                              definitions))
                                                       (when spec.max_tokens
                                                         (set body.max_completion_tokens
                                                              spec.max_tokens))
                                                       (let [request-options (misa.request-options.serialize serializer-id
                                                                                                             (or effect.request_options
                                                                                                                 {}))
                                                             headers [{:name :content-type
                                                                       :value :application/json}]]
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
                                                          :json (misa.patch body
                                                                            request-options)
                                                          :method :POST
                                                          :response_format :sse_json_stream
                                                          :timeouts spec.timeouts
                                                          :type :http/request
                                                          :url spec.url}))))})
                           (table.insert declarations
                                         {:catalog :events
                                          :value {:event (.. :provider/ spec.id
                                                             :-complete)
                                                  :handler stream}})
                           (definitions.build (.. :protocol.openai/ spec.id)
                             declarations
                             {}))))]
       (each [_ projection (ipairs delta-projections)]
         (table.insert declarations
                       {:catalog :openai-deltas
                        :id projection.id
                        :value projection.project}))
       (table.insert declarations
                     {:catalog :validators
                      :id :openai-deltas
                      :value (fn [_ value]
                               (assert (= (type value) :function)
                                       "OpenAI deltas require functions"))})
       {: configure
        : messages
        :definitions (definitions.build :protocol.openai declarations {})}))))
