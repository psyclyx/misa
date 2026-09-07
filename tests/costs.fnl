{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :priced/model
                                 :label "Friendly hidden name"
                                 :model :model
                                 :pricing {:cache_read 0.5
                                           :cache_write 3
                                           :input 2
                                           :output 8}
                                 :provider :priced}})
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :unknown/model
                                 :model :model
                                 :provider :unknown}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (fn close [a b]
                                      (assert (< (math.abs (- a b)) 1e-09)
                                              (.. (tostring a) " ~= " b))
                                      nil)

                                    (local inclusive
                                           (misa.cost_estimate {:cache_read 0.5
                                                                :input 2
                                                                :output 8}
                                                               {:cache_read_tokens 600
                                                                :input_includes_cache true
                                                                :input_tokens 1000
                                                                :output_tokens 100}))
                                    (close inclusive.usd 0.0019)
                                    (assert (and inclusive.estimated
                                                 (not inclusive.unknown)))
                                    (local exclusive
                                           (misa.cost_estimate {:cache_read 0.5
                                                                :input 2
                                                                :output 8}
                                                               {:cache_read_tokens 600
                                                                :input_includes_cache false
                                                                :input_tokens 400
                                                                :output_tokens 100}))
                                    (close exclusive.usd inclusive.usd)
                                    (assert (. (misa.cost_estimate {:input 2
                                                                    :output 8}
                                                                   {:cache_read_tokens 20
                                                                    :input_tokens 100})
                                               :unknown)
                                            "missing cache prices invented a cost")
                                    (assert (= (. (misa.cost_estimate nil
                                                                      {:cost_usd 0})
                                                  :usd)
                                               0)
                                            "free reported usage was treated as unknown")
                                    (local matches
                                           (misa.command_completions (misa.command :/model)
                                                                     "" db))
                                    (assert (and (= (. matches 1 :display
                                                       :label)
                                                    :priced/model)
                                                 (= (. matches 1 :display
                                                       :description)
                                                    nil))
                                            "friendly model name leaked into display")
                                    (local session
                                           (misa.choice_session {:items matches
                                                                 :query "friendly hidden"
                                                                 :title :Models}
                                                                db))
                                    (assert (and (= (length (. session.panels 1
                                                               :items))
                                                    1)
                                                 (= (. session.panels 1 :items
                                                       1 :id)
                                                    :priced/model))
                                            "friendly name stopped matching search")
                                    (assert (= (. (misa.model_cost_info db :priced/model) :pricing :input) 2)
                                            "model info lost configured rates")
                                    (assert (= (. (misa.model_cost_info db
                                                                        :unknown/model)
                                                  :unavailable)
                                               true))
                                    (local fx {})

                                    (fn dispatch [event]
                                      (tset fx (+ (length fx) 1)
                                            {: event :type :dispatch})
                                      nil)

                                    (dispatch {:model :priced/model
                                               :response_id :a
                                               :type :transcript/response-start})
                                    (dispatch {:models [{:id :priced/model
                                                         :pricing {:input 200
                                                                   :output 800}}]
                                               :provider :priced
                                               :type :models/update})
                                    (dispatch {:response_id :a
                                               :type :transcript/response-end
                                               :usage {:input_tokens 1000
                                                       :output_tokens 100}})
                                    (dispatch {:model :unknown/model
                                               :response_id :b
                                               :type :transcript/response-start})
                                    (dispatch {:response_id :b
                                               :type :transcript/response-end
                                               :usage {:cost_usd 0.012}})
                                    (dispatch {:response_id :b
                                               :type :transcript/response-end
                                               :usage {:cost_usd 0.012}})
                                    (dispatch {:type :test/costs-check})
                                    { : fx})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/costs-check
                         :handler (fn [db]
                                    (local first
                                           (misa.response_cost_projection db :a))
                                    (assert (= first.type :money))
                                    (assert (= first.currency :USD))
                                    (assert (= first.text nil))
                                    (assert (< (math.abs (- first.amount 0.0028))
                                               1e-09)
                                            "response rates changed after model catalogue refresh")
                                    (assert (and first.estimated
                                                 (not first.unknown)))
                                    (local second
                                           (misa.response_cost_projection db :b))
                                    (assert (and (and (= second.amount 0.012)
                                                      (not second.estimated))
                                                 (not second.unknown))
                                            "reported cost did not override missing rates")
                                    (local total (misa.sub db [:costs/total]))
                                    (assert (and (= total.responses 2)
                                                 (< (math.abs (- total.usd
                                                                 0.0148))
                                                    1e-09))
                                            "cost response replay was counted twice")
                                    {
                                     :fx [{:event {:type :transcript/reset}
                                           :type :dispatch}
                                          {:event {:type :test/costs-reset}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/costs-reset
                         :handler (fn [db]
                                    (assert (and (= (. (misa.sub db [:costs/total])
                                                       :usd)
                                                    0)
                                                 (= (. (misa.sub db [:costs/total])
                                                       :responses)
                                                    0))
                                            "clear retained conversation cost")
                                    {
                                     :fx [{:lines [{:spans [{:text :costs}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
