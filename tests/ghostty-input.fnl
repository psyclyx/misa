{:setup (fn [context]
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/model
                         :value {:id :smoke/model
                                 :label "Friendly name"
                                 :model :model
                                 :provider :smoke}})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (if (and (and (= tx.event.type
                                                              :terminal/input)
                                                           (= tx.event.kind
                                                              :alt))
                                                      (= tx.event.text :z))
                                             (misa.patch tx {:event (misa.replace {:type :smoke/inspect})})
                                             tx))
                                 :id :smoke/inspect}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :smoke/inspect
                         :handler (fn [db]
                                    (local inspections (+ (or db.inspections 0) 1))
                                    (local value
                                           {:attachments (length (or db.editor.attachments
                                                                     {}))
                                            :history db.history.entries
                                            :messages (length db.agent.messages)
                                            :pending (and db.queue
                                                          db.queue.pending)
                                            :picker (and db.picker
                                                         {:count (length db.picker.session.items)
                                                          :id db.picker.id})
                                            :requests (or db.requests {})
                                            :selection (not= db.selection nil)
                                            :status db.agent.status
                                            :text db.editor.text
                                            :top db.messages.top})
                                    {:patch {: inspections}
                                     :fx [{:completion :smoke/written
                                           :content (misa.json.encode value)
                                           :id (.. :snapshot- inspections)
                                           :path (.. context.config.smoke.directory
                                                     :/snapshot- inspections)
                                           :type :file/write}
                                          {:type :terminal/read}]})})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :provider.smoke
                         :handler (fn [effect]
                                    {:event {:id effect.id
                                             :messages effect.messages
                                             :type :smoke/request}
                                     :type :dispatch})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :smoke/request
                         :handler (fn [db event]
                                    (local requests (icollect [_ request (ipairs (or db.requests []))] request))
                                    (tset requests
                                          (+ (length requests) 1)
                                          (. event.messages
                                             (length event.messages)))
                                    {:patch {:requests (misa.replace requests)}
                                     :fx [{:argv [context.config.smoke.python
                                                  :-c
                                                  (.. "import time; time.sleep("
                                                      (or (and (= (length requests)
                                                                  1)
                                                               :2)
                                                          :0.1)
                                                      "); print('# Response\\n\\nA **bold** [link](https://example.test).\\n\\n' + 'Paragraph for scrolling.\\n\\n' * 20)")]
                                           :completion :smoke/response
                                           :id event.id
                                           :type :process/run}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :smoke/response
                         :handler (fn [_ event]
                                    (if (not event.ok)
                                        {:fx [{:event {:id event.id
                                                       :message :Cancelled
                                                       :type :agent/stream-error}
                                               :type :dispatch}]}
                                        {:fx [{:event {:id event.id
                                                       :type :agent/stream-start}
                                               :type :dispatch}
                                              {:event {:delta {:text event.stdout
                                                               :type :text}
                                                       :id event.id
                                                       :type :agent/stream-delta}
                                               :type :dispatch}
                                              {:event {:id event.id
                                                       :type :agent/stream-end
                                                       :usage {:input_tokens 10
                                                               :output_tokens 20}}
                                               :type :dispatch}]}))})
          nil
          {:fx setup-fx})}
