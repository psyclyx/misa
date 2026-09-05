;; Deterministic provider. Its cursor lives in its own model namespace.

{:setup (fn [context]
          (local setup-fx [])
          (local providers (or (and (= (type context.config) :table)
                                    context.config.providers)
                               nil))
          (local fake (or (and (= (type providers) :table) providers.fake) nil))
          (local responses (or (and (= (type fake) :table) fake.responses) nil))
          (assert (= (type responses) :table)
                  "config.providers.fake.responses must be an array of strings")
          (for [i 1 (length responses)]
            (assert (or (= (type (. responses i)) :string)
                        (= (type (. responses i)) :table))
                    "fake responses must be strings or content arrays"))
          (local serializer-id :fake.options)
          (table.insert setup-fx
                        {:type :register/request-options-serializer
                         :id serializer-id
                         :serializer {:accepts (fn [] true)
                                      :serialize (fn [target name value]
                                                   (tset target name value)
                                                   true)}})
          (local configured-models
                 (or (and (= (type fake.models) :table) fake.models)
                     [(or (and (= (type fake.model) :table) fake.model)
                          {:id :fake/default :label :Fake :model :default})]))
          (each [_ model (ipairs configured-models)]
            (var api model.api)
            (when (and (= (type api) :table)
                       (= (type api.request_options) :table))
              (local copy {})
              (each [key value (pairs api)] (tset copy key value))
              (set copy.request_options_serializer serializer-id)
              (set api copy))
            (table.insert setup-fx
                          {:type :register/model
                           :value {: api
                                   :context_window model.context_window
                                   :id model.id
                                   :label (or model.label model.id)
                                   :model model.model
                                   :provider :fake}}))
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.fake
                         :handler (fn [effect]
                                    (assert (and (= (type effect.messages)
                                                    :table)
                                                 (> (length effect.messages) 0))
                                            "fake messages must be nonempty")
                                    (assert (and (= (type effect.id) :string)
                                                 (not= effect.id ""))
                                            "fake id must be a nonempty string")
                                    (local transported
                                           (misa.serialize_request_options serializer-id
                                                                           (or effect.request_options
                                                                               {})
                                                                           {}))
                                    (when (= (type fake.expect_request_options)
                                             :table)
                                      (var count 0)
                                      (each [name value (pairs fake.expect_request_options)]
                                        (set count (+ count 1))
                                        (assert (= (. transported name) value)
                                                (.. "unexpected fake request option: "
                                                    name)))
                                      (when (= fake.expect_request_options_exact
                                               true)
                                        (var actual 0)
                                        (each [_ (pairs transported)]
                                          (set actual (+ actual 1)))
                                        (assert (= actual count)
                                                "unexpected additional fake request options")))
                                    {:event {:id effect.id
                                             :type :provider/fake}
                                     :type :dispatch})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :provider/fake
                         :handler (fn [db event]
                                    (assert (and (= (type event.id) :string)
                                                 (not= event.id ""))
                                            "fake id must be a nonempty string")
                                    (set db.providers (or db.providers {}))
                                    (local state
                                           (or db.providers.fake
                                               {:next_response 1}))
                                    (local response
                                           (. responses state.next_response))
                                    (set state.next_response
                                         (+ state.next_response 1))
                                    (set db.providers.fake state)
                                    (if (= response nil)
                                        {: db
                                         :fx [{:event {:id event.id
                                                       :message "fake responses exhausted"
                                                       :type :agent/stream-error}
                                               :type :dispatch}]}
                                        (do
                                          (var (chunks usage failure) nil)
                                          (if (and (= (type response) :table)
                                                   (= (type response.stream)
                                                      :table))
                                              (set (chunks usage failure)
                                                   (values response.stream
                                                           response.usage
                                                           response.error))
                                              (set chunks
                                                   (or (and (= (type response)
                                                               :string)
                                                            [{:text response
                                                              :type :text}])
                                                       response)))
                                          (local fx
                                                 [{:event {:id event.id
                                                           :type :agent/stream-start}
                                                   :type :dispatch}])
                                          (each [index chunk (ipairs chunks)]
                                            (assert (and (= (type chunk) :table)
                                                         (= (type chunk.type)
                                                            :string))
                                                    "fake stream chunks must be normalized deltas")
                                            (var delta chunk)
                                            (when (and (= chunk.type :tool_call)
                                                       (= chunk.index nil))
                                              (set delta {})
                                              (each [key value (pairs chunk)]
                                                (tset delta key value))
                                              (set delta.index index))
                                            (tset fx (+ (length fx) 1)
                                                  {:event {: delta
                                                           :id event.id
                                                           :type :agent/stream-delta}
                                                   :type :dispatch}))
                                          (tset fx (+ (length fx) 1)
                                                {:event (or (and failure
                                                                 {:id event.id
                                                                  :message failure
                                                                  :type :agent/stream-error})
                                                            {:id event.id
                                                             :type :agent/stream-end
                                                             : usage})
                                                 :type :dispatch})
                                          {: db : fx})))})
          {:fx setup-fx})}
