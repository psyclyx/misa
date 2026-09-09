(local definitions (require :tests.declarations))

(fn [context]
          (local declarations [])
          (table.insert declarations
                        (let [definition {:id :smoke/model
                                 :label "Friendly name"
                                 :model :model
                                 :provider :smoke}] {:catalog :models :id (. definition :id) :value definition}))
          (table.insert declarations
                        (let [definition {:id :smoke/inspect :event :terminal/input
                                 :priority 2000 :context [:db/path]
                                 :resolve (fn [_ input]
                                            (when (or (and (= input.kind :alt) (= input.text :z))
                                                      (and (= input.kind :key) (= input.key :alt+z)))
                                              {:type :smoke/inspect}))}] {:catalog :routes :id (. definition :id) :value definition}))
          (table.insert declarations
                        {:catalog :events  :value {:event :smoke/inspect :handler (fn [db]
                                    (local inspections (+ (or db.inspections 0) 1))
                                    (local value
                                           {:attachments (length (or db.editor.attachments
                                                                     {}))
                                            :history db.history.entries
                                            :hover_action db.hover_action
                                            :hover_link db.hover_link
                                            :messages (length db.agent.messages)
                                            :pending (and db.queue
                                                          db.queue.pending)
                                            :picker (and db.picker
                                                         {:count (length db.picker.session.items)
                                                          :id db.picker.id :combo db.picker.session.combo})
                                            :requests (or db.requests {})
                                            :selection (not= db.selection nil)
                                            :selection_visual (and db.selection (= db.selection.visual true))
                                            :selection_ranges (if db.selection (length (misa.selection.ranges db)) 0)
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
                                          {:type :terminal/read}]})}})
          (table.insert declarations
                        {:catalog :effects :id :provider.smoke :value (fn [effect]
                                    {:event {:id effect.id
                                             :messages effect.messages
                                             :type :smoke/request}
                                     :type :dispatch})})
          (table.insert declarations
                        {:catalog :events  :value {:event :smoke/request :handler (fn [db event]
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
                                                      "); print('# Response\\n\\nA **bold** [link](https://example.test).\\n\\n' + 'Paragraph for scrolling.\\n\\n' * 20 + '[footer](https://hover.test)')")]
                                           :completion :smoke/response
                                           :id event.id
                                           :type :process/run}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :smoke/response :handler (fn [_ event]
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
                                               :type :dispatch}]}))}})
          nil
          (definitions.collect :tests.ghostty-input declarations {}))
