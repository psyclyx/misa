;; Highlighting is requested by transcript changes, never by rendering.
(fn key-for [model]
  (local owner (tostring (or model.response_id "")))
  (.. (length owner) ":" owner (tostring model.id)))

(fn source-for [model]
  (or (and model.chunks (table.concat model.chunks)) model.text ""))

{:setup (fn [context]
          (local enabled (not= (. (or (. (or context.config {}) :messages) {})
                                  :markdown)
                               false))
          (fn request [state key index slot fx]
            (if (or slot.request slot.done) (values state slot)
                (let [next-id (+ state.next_id 1)
                      id (.. :syntax/ next-id)]
                  (table.insert fx
                            {:type :syntax/highlight
                             : id
                             :source slot.source
                             :language slot.language
                             :completion :syntax/completed})
                  (values (misa.patch state {:next_id next-id
                                            :pending {id {: key : index :source slot.source :language slot.language}}})
                          (misa.patch slot {:request id})))))

          (fn update-model [previous-state model fx]
            (var state previous-state)
            (when (and model.id
                       (or (= model.kind :assistant) (= model.kind :thinking)
                           (= model.kind :user) (= model.kind :harness)))
              (local key (key-for model))
              (local source (source-for model))
              (local old (. state.documents key))
              (when (or (not old) (not= old.source source))
                (local parsed (misa.markdown.parse source (and old old.document)))
                (local next {: source
                             :document parsed
                             :revision (+ (or (and old old.revision) 0) 1)
                             :slots {}})
                (each [_ block (ipairs parsed.blocks)]
                  (when (and (= block.kind :code_block)
                             (not= block.language "")
                             (<= (length block.language) 64)
                             (> (length block.text) 0)
                             (<= (length block.text) 1048576))
                    (local index (+ (length next.slots) 1))
                    (local previous (and old (. old.slots index)))
                    (local same
                           (and previous (= previous.source block.text)
                                (= previous.language block.language)))
                    (local previous-slot
                           (or (and same previous)
                               {:source block.text
                                :language block.language
                                :request (and previous previous.request)}))
                    (local (next-state slot)
                           (request state key index (misa.patch previous-slot {:start block.source_start}) fx))
                    (set state next-state)
                    (table.insert next.slots slot)))
                (set state (misa.patch state {:documents {key (misa.replace next)}}))))
            state)

          {:fx [{:type :register/sub
                 :value {:id :syntax/projections
                         :inputs [[:db/path :syntax :documents]]
                         :compute (fn [inputs _ previous]
                                    (local result {})
                                    (each [key entry (pairs (or (. inputs 1) {}))]
                                      (local old (and previous (. previous key)))
                                      (if (and old (= entry old.input))
                                          (tset result key old)
                                          (do
                                      (local captures {})
                                      (each [_ slot (ipairs entry.slots)]
                                        (when slot.data (tset captures slot.start slot.data)))
                                      (tset result key {:input entry
                                                        :value {:document entry.document : captures
                                                                :source entry.source :revision entry.revision}}))))
                                    result)}}
                {:type :register/sub
                 :value {:id :syntax/projection :inputs [[:syntax/projections]]
                         :compute (fn [inputs query]
                                    (local entry (. (. inputs 1) (. query 2)))
                                    (and entry entry.value))}}
                {:type :register/service
                 :name :syntax_projections
                 :value (fn [db] (misa.sub db [:syntax/projections]))}
                {:type :register/service
                 :name :syntax_projection
                 :value (fn [projections model]
                          (local key (key-for model))
                          (local source (source-for model))
                          (local entry (. projections key))
                          (local projection (and entry entry.value))
                          (when (and projection (= projection.source source)) projection))}
                {:type :register/event
                 :name :app/start
                 :handler (fn [db]
                            {:patch {:syntax (misa.replace {:next_id 0
                                  :pending {}
                                  :documents {}})}})}
                {:type :register/event
                 :name :transcript/reset
                 :handler (fn [db]
                            {:patch {:syntax {:pending (misa.replace {})
                                              :documents (misa.replace {})}}})}
                {:type :register/interceptor
                 :value {:id :syntax/transcript
                         :before (fn [tx]
                                   (misa.patch tx {:syntax_count
                                        (length (or (and tx.db.messages
                                                         tx.db.messages.blocks)
                                                    {}))}))
                         :after (fn [tx]
                                  (if (and enabled
                                             tx.cofx.terminal.interactive
                                             tx.db.syntax tx.db.messages)
                                    (do
                                    (local blocks tx.db.messages.blocks)
                                    (var state tx.db.syntax)
                                    (local effects (icollect [_ effect (ipairs tx.fx)] effect))
                                    (for [index (+ tx.syntax_count 1) (length blocks)]
                                      (set state (update-model state (. blocks index) effects)))
                                    (when (and tx.event.response_id
                                               (or (= tx.event.type
                                                      :transcript/block-delta)
                                                   (= tx.event.type
                                                      :transcript/block-end)
                                                   (= tx.event.type
                                                      :transcript/response-end)
                                                   (= tx.event.type
                                                      :transcript/response-interrupted)))
                                      (local position
                                             (. tx.db.messages.by_response
                                                tx.event.response_id))
                                      (local owner
                                             (and position
                                                  (. tx.db.messages.responses
                                                     position)))
                                      (when owner
                                        (for [index owner.block_start (- (+ owner.block_start
                                                                            owner.block_count)
                                                                         1)]
                                          (local model (. blocks index))
                                          (when (or (not tx.event.block_id)
                                                    (= model.id
                                                       tx.event.block_id))
                                            (set state (update-model state model effects))))))
                                    (misa.patch tx {:db {:syntax (misa.replace state)}
                                                    :fx (misa.replace effects)}))
                                    tx))}}
                {:type :register/event
                 :name :syntax/completed
                 :handler (fn [db event]
                            (local pending (. db.syntax.pending event.id))
                            (when pending
                              (var state (misa.patch db.syntax {:pending {event.id misa.delete}}))
                              (local entry (. state.documents pending.key))
                              (local previous (and entry (. entry.slots pending.index)))
                              (local fx {})
                              (when (and previous (= previous.request event.id))
                                (var slot (misa.patch previous {:request misa.delete}))
                                (var revision entry.revision)
                                (if (and (= slot.source pending.source) (= slot.language pending.language))
                                    (do
                                      (set slot (misa.patch slot {:done true}))
                                      (when (and event.ok (= (type event.data) :table))
                                        ;; Patch validation owns the capture payload; no external
                                        ;; cache may change the meaning of a retained state.
                                        (set slot (misa.patch slot {:data (misa.replace event.data)}))
                                        (set revision (+ revision 1))
                                        (table.insert fx {:type :dispatch :event {:type :ui/redraw}})))
                                    (let [(next-state next-slot) (request state pending.key pending.index slot fx)]
                                      (set state next-state)
                                      (set slot next-slot)))
                                (local slots {})
                                (each [index value (ipairs entry.slots)]
                                  (tset slots index (if (= index pending.index) slot value)))
                                (set state (misa.patch state
                                                       {:documents {pending.key
                                                                    (misa.replace (misa.patch entry
                                                                                              {:slots (misa.replace slots)
                                                                                               : revision}))}})))
                              {:patch {:syntax (misa.replace state)} : fx}))}]})}
