(local definitions (require :misa.definitions))

;; Record reducers keep opaque provider state independent of its presentation.
(fn emit [id kind data]
  {:type :dispatch :event (misa.patch (or data {}) {:id id :type kind})})

(fn delta [id value] (emit id :agent/stream-delta {:delta value}))
(fn provider-state [id provider value]
  (emit id :agent/stream-state {: provider : value}))

(local starts
       {:thinking (fn [_ record id]
                    (let [block record.content_block]
                      {:patch {:blocks {(tostring record.index) (misa.replace {:type :thinking
                                                                               :signature (or block.signature
                                                                                              "")
                                                                               :thinking (or block.thinking
                                                                                             "")})}}
                       :fx (if (and (= (type block.thinking) :string)
                                    (not= block.thinking ""))
                               [(delta id
                                       {:type :thinking :text block.thinking})]
                               [])}))
        :redacted_thinking (fn [_ record id provider]
                             {:fx [(provider-state id provider
                                                   record.content_block)]})
        :text (fn [_ record id]
                (let [text record.content_block.text]
                  (when (and (= (type text) :string) (not= text ""))
                    {:fx [(delta id {:type :text : text})]})))
        :tool_use (fn [_ record id]
                    {:fx [(delta id
                                 {:type :tool_call
                                  :arguments_json ""
                                  :id record.content_block.id
                                  :name record.content_block.name
                                  :index record.index})]})})

(local deltas
       {:text_delta (fn [_ record id]
                      {:fx [(delta id
                                   {:type :text
                                    :text (or record.delta.text "")})]})
        :thinking_delta (fn [state record id]
                          (let [key (tostring record.index)
                                block (. state.blocks key)
                                text (or record.delta.thinking "")]
                            {:patch (when block
                                      {:blocks {key {:thinking (.. block.thinking
                                                                   text)}}})
                             :fx [(delta id {:type :thinking : text})]}))
        :signature_delta (fn [state record]
                           (let [key (tostring record.index)
                                 block (. state.blocks key)]
                             (when block
                               {:patch {:blocks {key {:signature (.. block.signature
                                                                     (or record.delta.signature
                                                                         ""))}}}})))
        :input_json_delta (fn [_ record id]
                            {:fx [(delta id
                                         {:type :tool_call
                                          :index record.index
                                          :arguments_json_delta (or record.delta.partial_json
                                                                    "")})]})})

(fn nested [registry field]
  (fn [state record id provider]
    (let [value (. record field)
          handler (and (= (type value) :table)
                       (. (misa.catalog registry) value.type))]
      (when handler (handler state record id provider)))))

(local records
       {:message_stop (fn [] {:patch {:terminal true} :finish true})
        :content_block_start (nested :anthropic-block-starts :content_block)
        :content_block_delta (nested :anthropic-block-deltas :delta)
        :content_block_stop (fn [state record id provider]
                              (let [key (tostring record.index)
                                    block (. state.blocks key)]
                                {:patch {:blocks {key misa.delete}}
                                 :fx (if (and block (not= block.signature ""))
                                         [(provider-state id provider block)]
                                         [])}))
        :message_start (fn [_ record id]
                         (when (= (type record.message) :table)
                           (let [usage (if (= (type record.message.usage)
                                              :table)
                                           record.message.usage
                                           {})]
                             {:fx [(emit id :agent/stream-usage
                                         {:usage {:cache_read_tokens (or usage.cache_read_input_tokens
                                                                         0)
                                                  :cache_write_tokens (or usage.cache_creation_input_tokens
                                                                          0)
                                                  :input_includes_cache false
                                                  :input_tokens (or usage.input_tokens
                                                                    0)
                                                  :output_tokens (or usage.output_tokens
                                                                     0)}})]})))
        :message_delta (fn [_ record id]
                         (let [usage (if (= (type record.usage) :table)
                                         record.usage {})]
                           {:fx [(emit id :agent/stream-usage
                                       {:stop_reason (and (= (type record.delta)
                                                             :table)
                                                          record.delta.stop_reason)
                                        :usage {:output_tokens (or usage.output_tokens
                                                                   0)}})]}))
        :error (fn [_ record id]
                 {:patch {:failed true}
                  :finish true
                  :fx [(emit id :agent/stream-error
                             {:message (tostring (or (and (= (type record.error)
                                                             :table)
                                                          record.error.message)
                                                     "Anthropic request failed"))})]})})

(fn stream-update [id state fx]
  {:patch {:providers {:anthropic_streams {id (misa.replace state)}}}
   :fx (misa.stream.effects fx)})

(fn stream [provider db event]
  (let [state (or (and db.providers db.providers.anthropic_streams
                       (. db.providers.anthropic_streams event.id))
                  {:blocks {}})]
    (if (= event.phase :start)
        (stream-update event.id {:blocks {}}
                       [(emit event.id :agent/stream-start)])
        (= event.phase :end)
        (let [body (when (and (= (type event.body) :string)
                              (not= event.body ""))
                     event.body)
              http (when (and (= (type event.status) :number)
                              (>= event.status 400))
                     (.. "HTTP " event.status (if body (.. ": " body) "")))
              mismatch (= event.message :CredentialProfileMismatch)
              next-event (if (and event.ok state.terminal)
                             (emit event.id :agent/stream-end)
                             (emit event.id :agent/stream-error
                                   {:message (or (when mismatch
                                                   "credential endpoint profile changed; run /login kimi-coding for the selected region")
                                                 http body event.message
                                                 (when (not state.terminal)
                                                   "Anthropic stream ended without message_stop")
                                                 "Anthropic request failed")}))
              fx (if state.failed [] [next-event])]
          (when mismatch
            (table.insert fx
                          {:type :dispatch
                           :event {:type :models/provider-availability
                                   :available false
                                   :provider provider
                                   :reason :relogin-required}}))
          (stream-update event.id nil fx))
        (or state.terminal state.failed)
        nil
        (do
          (var next-state state)
          (let [fx []]
            (var finished false)
            (each [_ record (ipairs (or event.records [])) &until finished]
              (let [handler (. (misa.catalog :anthropic-records) record.type)
                    result (and handler
                                (handler next-state record event.id provider))]
                (when result
                  (set next-state (misa.patch next-state (or result.patch {})))
                  (each [_ effect (ipairs (or result.fx []))]
                    (table.insert fx effect))
                  (when result.finish
                    (set finished true)
                    (table.insert fx {:id event.id :type :operation/finish})))))
            (stream-update event.id next-state fx))))))

((fn [context]
   (let [declarations []]
     ;; Anthropic Messages protocol adapter shared by provider declarations.

     (fn message-content [message provider]
       (let [blocks {}]
         (each [_ state (ipairs (or message.provider_state {}))]
           (when (= state.provider (or provider :anthropic))
             (tset blocks (+ (length blocks) 1) state.value)))
         (each [_ block (ipairs (or message.content {}))]
           (if (= block.type :text)
               (tset blocks (+ (length blocks) 1)
                     {:text block.text :type :text})
               (= block.type :image)
               (tset blocks (+ (length blocks) 1)
                     {:source block.source :type :image})
               (= block.type :thinking)
               ;; Signed thinking is replayed from opaque protocol state above.
               (= block.type :tool_call)
               (do
                 (assert (= (type block.arguments) :table)
                         "canonical tool arguments must be a table")
                 (tset blocks (+ (length blocks) 1)
                       {:id block.id
                        :input block.arguments
                        :name block.name
                        :type :tool_use}))))
         blocks))

     (fn messages [items provider]
       "Serialize conversation messages for the Anthropic protocol."
       (let [result {}]
         (each [_ message (ipairs items)]
           (if (or (= message.role :user) (= message.role :assistant))
               (tset result (+ (length result) 1)
                     {:content (message-content message provider)
                      :role message.role})
               (= message.role :tool)
               (tset result (+ (length result) 1)
                     {:content [{:content (message-content message provider)
                                 :is_error (= message.is_error true)
                                 :tool_use_id message.tool_call_id
                                 :type :tool_result}]
                      :role :user})))
         result))

     (fn tools [items]
       (let [result {}]
         (each [_ tool (ipairs items)]
           (tset result (+ (length result) 1)
                 {:description tool.description
                  :input_schema tool.input_schema
                  :name tool.name}))
         result))

     (table.insert declarations
                   {:catalog :services
                    :id :protocols.anthropic-messages
                    :value messages})
     (let [configure (fn [spec]
                       "Describe this transport for a provider configuration."
                       (let [declarations []]
                         (assert (and (= (type spec.id) :string)
                                      (= (type spec.url) :string)
                                      (= (type spec.models) :table)))
                         (let [serializer-id (.. :anthropic.messages. spec.id)]
                           (table.insert declarations
                                         {:catalog :serializers
                                          :id serializer-id
                                          :value {:accepts (fn [name]
                                                             (= name
                                                                :reasoning_effort))
                                                  :serialize (fn [name value]
                                                               "Describe Anthropic request fields for a supported option."
                                                               (when (= name
                                                                        :reasoning_effort)
                                                                 {:output_config {:effort value}}))}})

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
                             (let [completion (.. :provider/ spec.id :-models)]
                               (fn request [after-id]
                                 (let [separator (or (and (spec.models_url:find "?"
                                                                                1
                                                                                true)
                                                          "&")
                                                     "?")]
                                   (var url
                                        (.. spec.models_url separator
                                            :limit=1000))
                                   (when after-id
                                     (assert (after-id:match "^[%w._:-]+$")
                                             "invalid Anthropic model cursor")
                                     (set url (.. url :&after_id= after-id)))
                                   (let [headers [{:name :anthropic-version
                                                   :value :2023-06-01}]]
                                     (each [_ header (ipairs (or spec.model_headers
                                                                 spec.headers {}))]
                                       (tset headers (+ (length headers) 1)
                                             header))
                                     {: completion
                                      :credential {:header (or spec.auth_header
                                                               :x-api-key)
                                                   :id spec.credential
                                                   :prefix (or spec.auth_prefix
                                                               "")}
                                      : headers
                                      :id (.. :models- spec.id)
                                      :method :GET
                                      :response_format :json
                                      :timeouts spec.timeouts
                                      :type :http/request
                                      : url})))

                               (fn finished [effects]
                                 (table.insert effects
                                               {:type :dispatch
                                                :event {:type :models/discovery-complete
                                                        :provider spec.id}})
                                 {:patch {:model_discovery {spec.id misa.delete}}
                                  :fx effects})

                               (fn discover [_]
                                 {:patch {:model_discovery {spec.id (misa.replace {})}}
                                  :fx [(request nil)]})

                               (fn discovered-model [item]
                                 {:api (model-api (or item.api
                                                      (when (= (type item.request_options)
                                                               :table)
                                                        {:request_options item.request_options})))
                                  :context_window (or item.context_window
                                                      item.context_length
                                                      (when (and (= (type item.max_input_tokens)
                                                                    :number)
                                                                 (> item.max_input_tokens
                                                                    0))
                                                        item.max_input_tokens))
                                  :id (.. spec.id "/" item.id)
                                  :label (or item.display_name item.name
                                             item.id)
                                  :model item.id})

                               (fn page [db event]
                                 (if (or (not event.ok)
                                         (not= (type event.data) :table)
                                         (not= (type event.data.data) :table))
                                     (finished [])
                                     (let [discovered []]
                                       (each [_ item (ipairs (or (and db.model_discovery
                                                                      (. db.model_discovery
                                                                         spec.id))
                                                                 []))]
                                         (table.insert discovered item))
                                       (each [_ item (ipairs event.data.data)]
                                         (when (and (= (type item) :table)
                                                    (= (type item.id) :string)
                                                    (or (not spec.model_filter)
                                                        (spec.model_filter item)))
                                           (table.insert discovered
                                                         (discovered-model item))))
                                       (if (= event.data.has_more true)
                                           (let [cursor event.data.last_id]
                                             (if (or (not= (type cursor)
                                                           :string)
                                                     (= cursor ""))
                                                 (finished [])
                                                 {:patch {:model_discovery {spec.id (misa.replace discovered)}}
                                                  :fx [(request cursor)]}))
                                           (let [effects []]
                                             (when (or (> (length discovered) 0)
                                                       (= spec.catalogue_authoritative
                                                          true))
                                               (table.sort discovered
                                                           (fn [left right]
                                                             (< left.id
                                                                right.id)))
                                               (table.insert effects
                                                             {:type :dispatch
                                                              :event {:type :models/replace-provider
                                                                      :provider spec.id
                                                                      :models discovered
                                                                      :authoritative (= spec.catalogue_authoritative
                                                                                        true)}}))
                                             (finished effects))))))

                               (table.insert declarations
                                             {:catalog :events
                                              :value {:event :models/discover
                                                      :handler (fn [db event]
                                                                 (when (or (not event.provider)
                                                                           (= event.provider
                                                                              spec.id))
                                                                   (discover db)))}})
                               (table.insert declarations
                                             {:catalog :events
                                              :value {:event completion
                                                      :handler page}})))
                           (table.insert declarations
                                         {:catalog :effects
                                          :id (.. :provider. spec.id)
                                          :value (fn [effect]
                                                   (let [body {:max_tokens (or spec.max_tokens
                                                                               16384)
                                                               :messages (messages effect.messages
                                                                                   spec.id)
                                                               :model effect.model
                                                               :stream true}]
                                                     (when effect.system_prompt
                                                       (set body.system
                                                            effect.system_prompt))
                                                     (when spec.thinking
                                                       (set body.thinking
                                                            spec.thinking))
                                                     (let [request-options (misa.request-options.serialize serializer-id
                                                                                                           (or effect.request_options
                                                                                                               {}))
                                                           definitions (tools effect.tools)]
                                                       (when (> (length definitions)
                                                                0)
                                                         (set body.tools
                                                              definitions))
                                                       (let [headers [{:name :content-type
                                                                       :value :application/json}
                                                                      {:name :anthropic-version
                                                                       :value :2023-06-01}]]
                                                         (each [_ header (ipairs (or spec.headers
                                                                                     {}))]
                                                           (tset headers
                                                                 (+ (length headers)
                                                                    1)
                                                                 header))
                                                         {:completion (.. :provider/
                                                                          spec.id
                                                                          :-complete)
                                                          :credential {:header (or spec.auth_header
                                                                                   :x-api-key)
                                                                       :id spec.credential
                                                                       :prefix (or spec.auth_prefix
                                                                                   "")}
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
                                                  :handler (fn [db event]
                                                             (stream spec.id db
                                                                     event))}})
                           (definitions.build (.. :protocol.anthropic/ spec.id)
                             declarations
                             {}))))]
       (each [id value (pairs records)]
         (table.insert declarations {:catalog :anthropic-records : id : value}))
       (table.insert declarations
                     {:catalog :validators
                      :id :anthropic-records
                      :value (fn [_ value]
                               (assert (= (type value) :function)
                                       "anthropic-records requires function definitions"))})
       (each [id value (pairs starts)]
         (table.insert declarations
                       {:catalog :anthropic-block-starts : id : value}))
       (table.insert declarations
                     {:catalog :validators
                      :id :anthropic-block-starts
                      :value (fn [_ value]
                               (assert (= (type value) :function)
                                       "anthropic-block-starts requires function definitions"))})
       (each [id value (pairs deltas)]
         (table.insert declarations
                       {:catalog :anthropic-block-deltas : id : value}))
       (table.insert declarations
                     {:catalog :validators
                      :id :anthropic-block-deltas
                      :value (fn [_ value]
                               (assert (= (type value) :function)
                                       "anthropic-block-deltas requires function definitions"))})
       {: configure
        : messages
        :definitions (definitions.build :protocol.anthropic declarations {})}))))
