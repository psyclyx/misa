;; Anthropic Messages protocol adapter shared by provider declarations.

(fn message-content [message provider]
  (let [blocks {}]
    (each [_ state (ipairs (or message.provider_state {}))]
      (when (= state.provider (or provider :anthropic))
        (tset blocks (+ (length blocks) 1) state.value)))
    (each [_ block (ipairs (or message.content {}))]
      (if (= block.type :text)
          (tset blocks (+ (length blocks) 1) {:text block.text :type :text})
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

(fn messages [___values___ provider]
  (let [result {}]
    (each [_ message (ipairs ___values___)]
      (if (or (= message.role :user) (= message.role :assistant))
          (tset result (+ (length result) 1)
                {:content (message-content message provider)
                 :role message.role}) (= message.role :tool)
          (tset result (+ (length result) 1)
                {:content [{:content (message-content message provider)
                            :is_error (= message.is_error true)
                            :tool_use_id message.tool_call_id
                            :type :tool_result}]
                 :role :user})))
    result))

(fn tools [___values___]
  (let [result {}]
    (each [_ tool (ipairs ___values___)]
      (tset result (+ (length result) 1)
            {:description tool.description
             :input_schema tool.input_schema
             :name tool.name}))
    result))

(set misa.protocols (or misa.protocols {}))

(set misa.protocols.serialize_anthropic_messages messages)

(fn misa.protocols.anthropic [spec]
  (assert (and (and (= (type spec.id) :string) (= (type spec.url) :string))
               (= (type spec.models) :table)))
  (local serializer-id (.. :anthropic.messages. spec.id))
  (misa.reg_request_options_serializer serializer-id
                                       {:accepts (fn [name]
                                                   (= name :reasoning_effort))
                                        :serialize (fn [body name value]
                                                     (if (not= name
                                                               :reasoning_effort)
                                                         false
                                                         (do
                                                           (set body.output_config
                                                                (or body.output_config
                                                                    {}))
                                                           (set body.output_config.effort
                                                                value)
                                                           true)))})

  (fn model-api [source]
    (if (not= (type source) :table) source
        (let [api {}]
          (each [key value (pairs source)] (tset api key value))
          (when (= (type api.request_options) :table)
            (set api.request_options_serializer serializer-id))
          api)))

  (each [_ model (ipairs spec.models)]
    (misa.reg_model {:api (model-api model.api)
                     :context_window model.context_window
                     :id model.id
                     :label (or model.label model.id)
                     :model model.model
                     :pricing model.pricing
                     :provider spec.id}))
  (when spec.models_url
    (local completion (.. :provider/ spec.id :-models))

    (fn request [after-id]
      (let [separator (or (and (spec.models_url:find "?" 1 true) "&") "?")]
        (var url (.. spec.models_url separator :limit=1000))
        (when after-id
          (assert (after-id:match "^[%w._:-]+$")
                  "invalid Anthropic model cursor")
          (set url (.. url :&after_id= after-id)))
        (local headers [{:name :anthropic-version :value :2023-06-01}])
        (each [_ header (ipairs (or (or spec.model_headers spec.headers) {}))]
          (tset headers (+ (length headers) 1) header))
        {: completion
         :credential {:header (or spec.auth_header :x-api-key)
                      :id spec.credential
                      :prefix (or spec.auth_prefix "")}
         : headers
         :id (.. :models- spec.id)
         :method :GET
         :response_format :json
         :timeouts spec.timeouts
         :type :http/request
         : url}))

    (fn discover [db]
      (set db.model_discovery (or db.model_discovery {}))
      (tset db.model_discovery spec.id {})
      {: db :fx [(request nil)]})

    (misa.reg_event :models/discover
                    (fn [db event]
                      (if (and event.provider (not= event.provider spec.id))
                          nil (discover db))))
    (misa.reg_event completion
                    (fn [db event]
                      (if (or (or (not event.ok)
                                  (not= (type event.data) :table))
                              (not= (type event.data.data) :table))
                          (do
                            (when db.model_discovery
                              (tset db.model_discovery spec.id nil))
                            {: db
                             :fx [{:event {:provider spec.id
                                           :type :models/discovery-complete}
                                   :type :dispatch}]})
                          (do
                            (set db.model_discovery (or db.model_discovery {}))
                            (local discovered
                                   (or (. db.model_discovery spec.id) {}))
                            (tset db.model_discovery spec.id discovered)
                            (each [_ item (ipairs event.data.data)]
                              (when (and (and (= (type item) :table)
                                              (= (type item.id) :string))
                                         (or (not spec.model_filter)
                                             (spec.model_filter item)))
                                (tset discovered (+ (length discovered) 1)
                                      {:api (model-api (or item.api
                                                           (or (and (= (type item.request_options)
                                                                       :table)
                                                                    {:request_options item.request_options})
                                                               nil)))
                                       :context_window (or (or item.context_window
                                                               item.context_length)
                                                           (or (and (and (= (type item.max_input_tokens)
                                                                            :number)
                                                                         (> item.max_input_tokens
                                                                            0))
                                                                    item.max_input_tokens)
                                                               nil))
                                       :id (.. spec.id "/" item.id)
                                       :label (or (or item.display_name
                                                      item.name)
                                                  item.id)
                                       :model item.id})))
                            (if (= event.data.has_more true)
                                (let [cursor event.data.last_id]
                                  (if (or (not= (type cursor) :string)
                                          (= cursor ""))
                                      (do
                                        (tset db.model_discovery spec.id nil)
                                        {: db
                                         :fx [{:event {:provider spec.id
                                                       :type :models/discovery-complete}
                                               :type :dispatch}]})
                                      {: db :fx [(request cursor)]}))
                                (do
                                  (tset db.model_discovery spec.id nil)
                                  (local effects
                                         [{:event {:provider spec.id
                                                   :type :models/discovery-complete}
                                           :type :dispatch}])
                                  (when (or (> (length discovered) 0)
                                            (= spec.catalogue_authoritative
                                               true))
                                    (table.sort discovered
                                                (fn [left right]
                                                  (< left.id right.id)))
                                    (table.insert effects 1
                                                  {:event {:authoritative (= spec.catalogue_authoritative
                                                                             true)
                                                           :models discovered
                                                           :provider spec.id
                                                           :type :models/replace-provider}
                                                   :type :dispatch}))
                                  {: db :fx effects})))))))
  (misa.reg_fx (.. :provider. spec.id)
               (fn [effect]
                 (let [body {:max_tokens (or spec.max_tokens 16384)
                             :messages (messages effect.messages spec.id)
                             :model effect.model
                             :stream true}]
                   (when effect.system_prompt
                     (set body.system effect.system_prompt))
                   (when spec.thinking (set body.thinking spec.thinking))
                   (misa.serialize_request_options serializer-id
                                                   (or effect.request_options
                                                       {})
                                                   body)
                   (local definitions (tools effect.tools))
                   (when (> (length definitions) 0)
                     (set body.tools definitions))
                   (local headers
                          [{:name :content-type :value :application/json}
                           {:name :anthropic-version :value :2023-06-01}])
                   (each [_ header (ipairs (or spec.headers {}))]
                     (tset headers (+ (length headers) 1) header))
                   {:completion (.. :provider/ spec.id :-complete)
                    :credential {:header (or spec.auth_header :x-api-key)
                                 :id spec.credential
                                 :prefix (or spec.auth_prefix "")}
                    : headers
                    :id effect.id
                    :json body
                    :method :POST
                    :response_format :sse_json_stream
                    :timeouts spec.timeouts
                    :type :http/request
                    :url spec.url})))
  (misa.reg_event (.. :provider/ spec.id :-complete)
                  (fn [db event]
                    (set db.providers (or db.providers {}))
                    (set db.providers.anthropic_streams
                         (or db.providers.anthropic_streams {}))
                    (local streams db.providers.anthropic_streams)
                    (if (= event.phase :start)
                        (do
                          (tset streams event.id {:blocks {}})
                          {: db
                           :fx [{:event {:id event.id
                                         :type :agent/stream-start}
                                 :type :dispatch}]})
                        (let [state (or (. streams event.id) {:blocks {}})]
                          (if (= event.phase :end)
                              (let [terminal (= state.terminal true)]
                                (tset streams event.id nil)
                                (local body
                                       (or (and (and (= (type event.body)
                                                        :string)
                                                     (not= event.body ""))
                                                event.body)
                                           nil))
                                (local http
                                       (or (and (and (= (type event.status)
                                                        :number)
                                                     (>= event.status 400))
                                                (.. "HTTP "
                                                    (tostring event.status)
                                                    (or (and body
                                                             (.. ": " body))
                                                        "")))
                                           nil))
                                (local mismatch
                                       (= event.message
                                          :CredentialProfileMismatch))
                                (local next-event
                                       (or (and (and event.ok terminal)
                                                {:id event.id
                                                 :type :agent/stream-end})
                                           {:id event.id
                                            :message (or (or (or (or (or (and mismatch
                                                                              "credential endpoint profile changed; run /login kimi-coding for the selected region")
                                                                         http)
                                                                     body)
                                                                 event.message)
                                                             (and (not terminal)
                                                                  "Anthropic stream ended without message_stop"))
                                                         "Anthropic request failed")
                                            :type :agent/stream-error}))
                                (local fx [{:event next-event :type :dispatch}])
                                (when mismatch
                                  (tset fx (+ (length fx) 1)
                                        {:event {:available false
                                                 :provider spec.id
                                                 :reason :relogin-required
                                                 :type :models/provider-availability}
                                         :type :dispatch}))
                                {: db : fx})
                              (do
                                (var (fx terminal) (values {} false))
                                (each [_ record (ipairs (or event.records {}))]
                                  (if (= record.type :message_stop)
                                      (do
                                        (set (state.terminal terminal)
                                             (values true true))
                                        (lua :break))
                                      (and (= record.type :content_block_start)
                                           (= (type record.content_block)
                                              :table))
                                      (let [block record.content_block]
                                        (if (= block.type :thinking)
                                            (do
                                              (tset state.blocks
                                                    (tostring record.index)
                                                    {:signature (or block.signature
                                                                    "")
                                                     :thinking (or block.thinking
                                                                   "")
                                                     :type :thinking})
                                              (when (and (= (type block.thinking)
                                                            :string)
                                                         (not= block.thinking
                                                               ""))
                                                (tset fx (+ (length fx) 1)
                                                      {:event {:delta {:text block.thinking
                                                                       :type :thinking}
                                                               :id event.id
                                                               :type :agent/stream-delta}
                                                       :type :dispatch})))
                                            (= block.type :redacted_thinking)
                                            (tset fx (+ (length fx) 1)
                                                  {:event {:id event.id
                                                           :provider spec.id
                                                           :type :agent/stream-state
                                                           :value block}
                                                   :type :dispatch})
                                            (and (and (= block.type :text)
                                                      (= (type block.text)
                                                         :string))
                                                 (not= block.text ""))
                                            (tset fx (+ (length fx) 1)
                                                  {:event {:delta {:text block.text
                                                                   :type :text}
                                                           :id event.id
                                                           :type :agent/stream-delta}
                                                   :type :dispatch})
                                            (= block.type :tool_use)
                                            (tset fx (+ (length fx) 1)
                                                  {:event {:delta {:arguments_json ""
                                                                   :id block.id
                                                                   :index record.index
                                                                   :name block.name
                                                                   :type :tool_call}
                                                           :id event.id
                                                           :type :agent/stream-delta}
                                                   :type :dispatch})))
                                      (and (= record.type :content_block_delta)
                                           (= (type record.delta) :table))
                                      (let [delta record.delta]
                                        (if (= delta.type :text_delta)
                                            (tset fx (+ (length fx) 1)
                                                  {:event {:delta {:text (or delta.text
                                                                             "")
                                                                   :type :text}
                                                           :id event.id
                                                           :type :agent/stream-delta}
                                                   :type :dispatch})
                                            (= delta.type :thinking_delta)
                                            (do
                                              (tset fx (+ (length fx) 1)
                                                    {:event {:delta {:text (or delta.thinking
                                                                               "")
                                                                     :type :thinking}
                                                             :id event.id
                                                             :type :agent/stream-delta}
                                                     :type :dispatch})
                                              (local block
                                                     (. state.blocks
                                                        (tostring record.index)))
                                              (when block
                                                (set block.thinking
                                                     (.. block.thinking
                                                         (or delta.thinking "")))))
                                            (= delta.type :signature_delta)
                                            (let [block (. state.blocks
                                                           (tostring record.index))]
                                              (when block
                                                (set block.signature
                                                     (.. block.signature
                                                         (or delta.signature "")))))
                                            (= delta.type :input_json_delta)
                                            (tset fx (+ (length fx) 1)
                                                  {:event {:delta {:arguments_json_delta (or delta.partial_json
                                                                                             "")
                                                                   :index record.index
                                                                   :type :tool_call}
                                                           :id event.id
                                                           :type :agent/stream-delta}
                                                   :type :dispatch})))
                                      (= record.type :content_block_stop)
                                      (let [block (. state.blocks
                                                     (tostring record.index))]
                                        (when (and block
                                                   (not= block.signature ""))
                                          (tset fx (+ (length fx) 1)
                                                {:event {:id event.id
                                                         :provider spec.id
                                                         :type :agent/stream-state
                                                         :value block}
                                                 :type :dispatch}))
                                        (tset state.blocks
                                              (tostring record.index) nil))
                                      (and (= record.type :message_start)
                                           (= (type record.message) :table))
                                      (let [usage (or (and (= (type record.message.usage)
                                                              :table)
                                                           record.message.usage)
                                                      {})]
                                        (tset fx (+ (length fx) 1)
                                              {:event {:id event.id
                                                       :type :agent/stream-usage
                                                       :usage {:cache_read_tokens (or usage.cache_read_input_tokens
                                                                                      0)
                                                               :cache_write_tokens (or usage.cache_creation_input_tokens
                                                                                       0)
                                                               :input_includes_cache false
                                                               :input_tokens (or usage.input_tokens
                                                                                 0)
                                                               :output_tokens (or usage.output_tokens
                                                                                  0)}}
                                               :type :dispatch}))
                                      (= record.type :message_delta)
                                      (let [usage (or (and (= (type record.usage)
                                                              :table)
                                                           record.usage)
                                                      {})]
                                        (tset fx (+ (length fx) 1)
                                              {:event {:id event.id
                                                       :stop_reason (or (and (= (type record.delta)
                                                                                :table)
                                                                             record.delta.stop_reason)
                                                                        nil)
                                                       :type :agent/stream-usage
                                                       :usage {:output_tokens (or usage.output_tokens
                                                                                  0)}}
                                               :type :dispatch}))
                                      (= record.type :error)
                                      (tset fx (+ (length fx) 1)
                                            {:event {:id event.id
                                                     :message (tostring (or (and (= (type record.error)
                                                                                    :table)
                                                                                 record.error.message)
                                                                            "Anthropic request failed"))
                                                     :type :agent/stream-error}
                                             :type :dispatch})))
                                (tset streams event.id state)
                                (when terminal
                                  (tset fx (+ (length fx) 1)
                                        {:id event.id :type :operation/finish}))
                                {: db : fx}))))))
  nil)

{:setup (fn [] nil)}

