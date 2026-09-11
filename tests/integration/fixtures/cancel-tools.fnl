(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (var (provider-calls cancel-started) (values 0 false))
  (each [_ name (ipairs [:slow_one :slow_two])]
    (table.insert declarations
                  (let [definition {:description name
                                    :effect :test/slow
                                    :input_schema {:additionalProperties false
                                                   :properties {}
                                                   :type :object}
                                    : name}]
                    {:catalog :tools
                     :id (. definition :name)
                     :value definition})))
  (table.insert declarations
                (let [definition {:id :cancel/model
                                  :model :model
                                  :provider :cancel}]
                  {:catalog :models :id (. definition :id) :value definition}))
  (table.insert declarations
                {:catalog :effects
                 :id :provider.cancel
                 :value (fn [effect]
                          (set provider-calls (+ provider-calls 1))
                          (assert (= provider-calls 1)
                                  "cancelled tools continued the model request")
                          {:event {:content [{:arguments {}
                                              :id :slow-1
                                              :name :slow_one
                                              :type :tool_call}
                                             {:arguments {}
                                              :id :slow-2
                                              :name :slow_two
                                              :type :tool_call}]
                                   :id effect.id
                                   :type :agent/result}
                           :type :dispatch})})
  (table.insert declarations
                {:catalog :effects
                 :id :test/slow
                 :value (fn [effect]
                          {:argv [:sh :-c "sleep 10"]
                           :completion :test/slow-complete
                           :id effect.tool_call_id
                           :type :process/run})})
  (table.insert declarations
                {:catalog :events
                 :value {:event :test/slow-complete
                         :handler (fn [_ event]
                                    {:fx [{:event {:is_error (not event.ok)
                                                   :text (or event.message
                                                             :done)
                                                   :tool_call_id event.id
                                                   :type :tool/result}
                                           :type :dispatch}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :agent/status
                         :handler (fn [_ event]
                                    (if (and (= event.status :tools)
                                             (not cancel-started))
                                        (do
                                          (set cancel-started true)
                                          {:fx [{:completion :test/cancel
                                                 :id :cancel-delay
                                                 :interval_ms 50
                                                 :type :timer/start}]})
                                        nil))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :test/cancel
                         :handler (fn []
                                    {:fx [{:id :cancel-delay :type :timer/stop}
                                          {:event {:type :agent/cancel-active}
                                           :type :dispatch}]})}})
  (local cancelled {})
  (table.insert declarations {:catalog :effects
                              :id :operation/cancel
                              :value (fn [effect]
                                       (tset cancelled effect.id true)
                                       effect)})
  (table.insert declarations
                {:catalog :events
                 :value {:event :agent/cancel-active
                         :handler (fn [db]
                                    (assert (and db.agent.cancel_requested
                                                 (= db.agent.status :cancelling))
                                            "tool cancellation intent was not recorded")
                                    nil)}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :agent/completed
                         :handler (fn [db]
                                    (if (not cancel-started) nil
                                        (do
                                          (assert (and (. cancelled :slow-1)
                                                       (. cancelled :slow-2))
                                                  "not every pending native tool call was cancelled")
                                          (assert (and (and (= db.agent.status
                                                               :ready)
                                                            (= db.agent.pending_tool_count
                                                               0))
                                                       (not db.agent.cancel_requested))
                                                  "cancelled tools did not return ready")
                                          (assert (and (= provider-calls 1)
                                                       (= (length db.agent.messages)
                                                          4))
                                                  "cancelled tool batch continued the model or left unmatched calls")
                                          (for [i 3 4]
                                            (assert (and (. db.agent.messages i
                                                            :is_error)
                                                         (= (. db.agent.messages
                                                               i :content 1
                                                               :text)
                                                            :Cancelled))
                                                    "cancelled calls need explicit history results"))
                                          (var (saw-cancel saw-interrupted
                                                           cancelled-sections)
                                               (values false false 0))
                                          (each [_ block (ipairs db.messages.blocks)]
                                            (set saw-cancel
                                                 (or saw-cancel
                                                     (and (= block.kind
                                                             :harness)
                                                          (= block.text
                                                             :Cancelled))))
                                            (set saw-interrupted
                                                 (or saw-interrupted
                                                     (= block.interrupted true)))
                                            (when (and (= block.kind :tool_call)
                                                       (= block.status
                                                          :cancelled))
                                              (set cancelled-sections
                                                   (+ cancelled-sections 1))))
                                          (assert (and (and saw-cancel
                                                            saw-interrupted)
                                                       (= cancelled-sections 2))
                                                  "cancelled tool-section state was not visible")
                                          {:fx [{:lines [{:spans [{:text "cancel tools"}]}]
                                                 :type :view/commit}
                                                {:type :app/quit}]})))}})
  nil
  (definitions.collect :tests.integration.fixtures.cancel-tools declarations {}))
