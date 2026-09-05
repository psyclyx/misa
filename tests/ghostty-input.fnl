{:setup (fn [context]
          (misa.reg_model {:id :smoke/model
                           :label "Friendly name"
                           :model :model
                           :provider :smoke})
          (misa.reg_interceptor {:before (fn [tx]
                                           (when (and (and (= tx.event.type
                                                              :terminal/input)
                                                           (= tx.event.kind
                                                              :alt))
                                                      (= tx.event.text :z))
                                             (set tx.event
                                                  {:type :smoke/inspect}))
                                           tx)
                                 :id :smoke/inspect})
          (misa.reg_event :smoke/inspect
                          (fn [db]
                            (set db.inspections (+ (or db.inspections 0) 1))
                            (local value
                                   {:attachments (length (or db.editor.attachments
                                                             {}))
                                    :history db.history.entries
                                    :messages (length db.agent.messages)
                                    :pending (and db.queue db.queue.pending)
                                    :picker (and db.picker
                                                 {:count (length db.picker.session.items)
                                                  :id db.picker.id})
                                    :requests (or db.requests {})
                                    :selection (not= db.selection nil)
                                    :status db.agent.status
                                    :text db.editor.text
                                    :top db.messages.top})
                            {: db
                             :fx [{:completion :smoke/written
                                   :content (misa.json.encode value)
                                   :id (.. :snapshot- db.inspections)
                                   :path (.. context.config.smoke.directory
                                             :/snapshot- db.inspections)
                                   :type :file/write}
                                  {:type :terminal/read}]}))
          (misa.reg_fx :provider.smoke
                       (fn [effect]
                         {:event {:id effect.id
                                  :messages effect.messages
                                  :type :smoke/request}
                          :type :dispatch}))
          (misa.reg_event :smoke/request
                          (fn [db event]
                            (set db.requests (or db.requests {}))
                            (tset db.requests (+ (length db.requests) 1)
                                  (. event.messages (length event.messages)))
                            {: db
                             :fx [{:argv [context.config.smoke.python
                                          :-c
                                          (.. "import time; time.sleep("
                                              (or (and (= (length db.requests)
                                                          1)
                                                       :2)
                                                  :0.1)
                                              "); print('# Response\\n\\nA **bold** [link](https://example.test).\\n\\n' + 'Paragraph for scrolling.\\n\\n' * 20)")]
                                   :completion :smoke/response
                                   :id event.id
                                   :type :process/run}]}))
          (misa.reg_event :smoke/response
                          (fn [_ event]
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
                                       :type :dispatch}]})))
          nil)}

