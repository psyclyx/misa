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
                                    (if (not= (type source) :table) source
                                        (let [api {}]
                                          (each [key value (pairs source)]
                                            (tset api key value))
                                          (when (= (type api.request_options)
                                                   :table)
                                            (set api.request_options_serializer
                                                 serializer-id))
                                          api)))

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
                                      (let [credential (or (and (= spec.models_credential
                                                                   false)
                                                                nil)
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
                                                 :name (.. :provider/ spec.id
                                                           :-complete)
                                                 :handler (fn [db event]
                                                            (set db.providers
                                                                 (or db.providers
                                                                     {}))
                                                            (set db.providers.openai_streams
                                                                 (or db.providers.openai_streams
                                                                     {}))
                                                            (local streams
                                                                   db.providers.openai_streams)
                                                            (local fx {})
                                                            (if (= event.phase
                                                                   :start)
                                                                (do
                                                                  (tset streams
                                                                        event.id
                                                                        false)
                                                                  {: db
                                                                   :fx [{:event {:id event.id
                                                                                 :type :agent/stream-start}
                                                                         :type :dispatch}]})
                                                                (= event.phase
                                                                   :end)
                                                                (let [terminal (= (. streams
                                                                                     event.id)
                                                                                  true)]
                                                                  (tset streams
                                                                        event.id
                                                                        nil)
                                                                  (local body
                                                                         (or (and (and (= (type event.body)
                                                                                          :string)
                                                                                       (not= event.body
                                                                                             ""))
                                                                                  event.body)
                                                                             nil))
                                                                  (local http
                                                                         (or (and (and (= (type event.status)
                                                                                          :number)
                                                                                       (>= event.status
                                                                                           400))
                                                                                  (.. "HTTP "
                                                                                      (tostring event.status)
                                                                                      (or (and body
                                                                                               (.. ": "
                                                                                                   body))
                                                                                          "")))
                                                                             nil))
                                                                  (local next-event
                                                                         (or (and (and event.ok
                                                                                       terminal)
                                                                                  {:id event.id
                                                                                   :type :agent/stream-end})
                                                                             {:id event.id
                                                                              :message (or (or (or (or http
                                                                                                       body)
                                                                                                   event.message)
                                                                                               (and (not terminal)
                                                                                                    "OpenAI stream ended without [DONE]"))
                                                                                           "OpenAI request failed")
                                                                              :type :agent/stream-error}))
                                                                  {: db
                                                                   :fx [{:event next-event
                                                                         :type :dispatch}]})
                                                                (do
                                                                  (when (= event.terminal
                                                                           true)
                                                                    (tset streams
                                                                          event.id
                                                                          true))
                                                                  (each [_ record (ipairs (or event.records
                                                                                              {}))]
                                                                    (local choice
                                                                           (or (and (= (type record.choices)
                                                                                       :table)
                                                                                    (. record.choices
                                                                                       1))
                                                                               nil))
                                                                    (local delta
                                                                           (or (and choice
                                                                                    choice.delta)
                                                                               nil))
                                                                    (when (= (type delta)
                                                                             :table)
                                                                      (when (and (= (type delta.content)
                                                                                    :string)
                                                                                 (not= delta.content
                                                                                       ""))
                                                                        (tset fx
                                                                              (+ (length fx)
                                                                                 1)
                                                                              {:event {:delta {:text delta.content
                                                                                               :type :text}
                                                                                       :id event.id
                                                                                       :type :agent/stream-delta}
                                                                               :type :dispatch}))
                                                                      (local thinking
                                                                             (or delta.reasoning_content
                                                                                 delta.reasoning))
                                                                      (when (and (= (type thinking)
                                                                                    :string)
                                                                                 (not= thinking
                                                                                       ""))
                                                                        (tset fx
                                                                              (+ (length fx)
                                                                                 1)
                                                                              {:event {:delta {:text thinking
                                                                                               :type :thinking}
                                                                                       :id event.id
                                                                                       :type :agent/stream-delta}
                                                                               :type :dispatch}))
                                                                      (each [_ call (ipairs (or delta.tool_calls
                                                                                                {}))]
                                                                        (local ___fn___
                                                                               (or (and (= (type (. call
                                                                                                    :function))
                                                                                           :table)
                                                                                        (. call
                                                                                           :function))
                                                                                   {}))
                                                                        (tset fx
                                                                              (+ (length fx)
                                                                                 1)
                                                                              {:event {:delta {:arguments_json_delta ___fn___.arguments
                                                                                               :id call.id
                                                                                               :index (or call.index
                                                                                                          0)
                                                                                               :name ___fn___.name
                                                                                               :type :tool_call}
                                                                                       :id event.id
                                                                                       :type :agent/stream-delta}
                                                                               :type :dispatch})))
                                                                    (var raw-usage
                                                                         (or (and (= (type record.usage)
                                                                                     :table)
                                                                                  record.usage)
                                                                             nil))
                                                                    (when (or raw-usage
                                                                              (and choice
                                                                                   choice.finish_reason))
                                                                      (set raw-usage
                                                                           (or raw-usage
                                                                               {}))
                                                                      (local details
                                                                             (or (and (= (type raw-usage.prompt_tokens_details)
                                                                                         :table)
                                                                                      raw-usage.prompt_tokens_details)
                                                                                 {}))
                                                                      (tset fx
                                                                            (+ (length fx)
                                                                               1)
                                                                            {:event {:id event.id
                                                                                     :stop_reason (and choice
                                                                                                       choice.finish_reason)
                                                                                     :type :agent/stream-usage
                                                                                     :usage {:cache_read_tokens (or details.cached_tokens
                                                                                                                    0)
                                                                                             :cache_write_tokens 0
                                                                                             :cost_usd raw-usage.cost
                                                                                             :input_includes_cache true
                                                                                             :input_tokens (or raw-usage.prompt_tokens
                                                                                                               0)
                                                                                             :output_tokens (or raw-usage.completion_tokens
                                                                                                                0)}}
                                                                             :type :dispatch})))
                                                                  (when (= event.terminal
                                                                           true)
                                                                    (tset fx
                                                                          (+ (length fx)
                                                                             1)
                                                                          {:id event.id
                                                                           :type :operation/finish}))
                                                                  {: db : fx})))})
                                  nil
                                  {:fx setup-fx})})
          {:fx setup-fx})}
