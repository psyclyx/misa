(local definitions (require :misa.definitions))

(fn request [fake serializer-id effect]
  (assert (and (= (type effect.messages) :table) (> (length effect.messages) 0))
          "fake messages must be nonempty")
  (assert (and (= (type effect.id) :string) (not= effect.id ""))
          "fake id must be a nonempty string")
  (let [transported (misa.request-options.serialize serializer-id
                                                    (or effect.request_options
                                                        {})
                                                    {})]
    (when (= (type fake.expect_request_options) :table)
      (var count 0)
      (each [name value (pairs fake.expect_request_options)]
        (set count (+ count 1))
        (assert (= (. transported name) value)
                (.. "unexpected fake request option: " name)))
      (when (= fake.expect_request_options_exact true)
        (var actual 0)
        (each [_ (pairs transported)]
          (set actual (+ actual 1)))
        (assert (= actual count) "unexpected additional fake request options")))
    {:event {:id effect.id :type :provider/fake} :type :dispatch}))

(fn respond [responses db event]
  "Advance the deterministic response cursor and emit its content."
  (assert (and (= (type event.id) :string) (not= event.id ""))
          "fake id must be a nonempty string")
  (let [state (or (and db.providers db.providers.fake) {:next_response 1})
        response (. responses state.next_response)
        patch {:providers {:fake {:next_response (+ state.next_response 1)}}}]
    (if (= response nil)
        {: patch
         :fx [{:event {:id event.id
                       :message "fake responses exhausted"
                       :type :agent/stream-error}
               :type :dispatch}]}
        (do
          (var (chunks usage failure) nil)
          (if (and (= (type response) :table) (= (type response.stream) :table))
              (set (chunks usage failure)
                   (values response.stream response.usage response.error))
              (set chunks (or (and (= (type response) :string)
                                   [{:text response :type :text}])
                              response)))
          (let [fx [{:event {:id event.id :type :agent/stream-start}
                     :type :dispatch}]]
            (each [index chunk (ipairs chunks)]
              (assert (and (= (type chunk) :table)
                           (= (type chunk.type) :string))
                      "fake stream chunks must be normalized deltas")
              (let [delta (if (and (= chunk.type :tool_call)
                                   (= chunk.index nil))
                              (misa.patch chunk {: index})
                              chunk)]
                (tset fx (+ (length fx) 1)
                      {:event {: delta :id event.id :type :agent/stream-delta}
                       :type :dispatch})))
            (tset fx (+ (length fx) 1)
                  {:event (or (and failure
                                   {:id event.id
                                    :message failure
                                    :type :agent/stream-error})
                              {:id event.id :type :agent/stream-end : usage})
                   :type :dispatch})
            {: patch : fx})))))

(fn build [context]
  "Build the fake provider catalogs from application settings."
  (let [declarations []
        providers (or (and (= (type context.config) :table)
                           context.config.providers) nil)
        fake (or (and (= (type providers) :table) providers.fake) nil)
        responses (or (and (= (type fake) :table) fake.responses) nil)]
    (assert (= (type responses) :table)
            "config.providers.fake.responses must be an array of strings")
    (for [i 1 (length responses)]
      (assert (or (= (type (. responses i)) :string)
                  (= (type (. responses i)) :table))
              "fake responses must be strings or content arrays"))
    (let [serializer-id :fake.options]
      (table.insert declarations
                    {:catalog :serializers
                     :id serializer-id
                     :value {:accepts (fn []
                                        true)
                             :serialize (fn [name value]
                                          "Return the requested option as data."
                                          {name value})}})
      (let [configured-models (or (and (= (type fake.models) :table)
                                       fake.models)
                                  [(or (and (= (type fake.model) :table)
                                            fake.model)
                                       {:id :fake/default
                                        :label :Fake
                                        :model :default})])]
        (each [_ model (ipairs configured-models)]
          (let [api (if (and (= (type model.api) :table)
                             (= (type model.api.request_options) :table))
                        (misa.patch model.api
                                    {:request_options_serializer serializer-id})
                        model.api)]
            (table.insert declarations
                          (let [definition {: api
                                            :context_window model.context_window
                                            :id model.id
                                            :label (or model.label model.id)
                                            :model model.model
                                            :provider :fake}]
                            {:catalog :models
                             :id (. definition :id)
                             :value definition}))))
        (table.insert declarations
                      {:catalog :effects
                       :id :provider.fake
                       :value (fn [effect]
                                (request fake serializer-id effect))})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :provider/fake
                               :handler (fn [db event]
                                          (respond responses db event))}})
        (definitions.build :provider.fake declarations {})))))

{:build build :respond respond}
