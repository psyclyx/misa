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
          (local documents {})
          (local dirty {})
          (var cache-epoch 0)

          (fn prune-results [cached entry]
            (local keep {})
            (each [_ slot (ipairs (or (and entry entry.slots) {}))]
              (when slot.result (tset keep slot.result true)))
            (each [id (pairs cached.results)]
              (when (not (. keep id)) (tset cached.results id nil))))

          (fn document [key source]
            (var cached (. documents key))
            (when (not cached)
              (set cached {:parser (misa.markdown.new_document) :results {}})
              (tset documents key cached))
            (when (not= cached.source source)
              (set cached.document (cached.parser:update source))
              (set cached.source source))
            cached.document)

          (fn request [state key index slot fx]
            (when (and (not slot.request) (not slot.done))
              (set state.next_id (+ state.next_id 1))
              (local id (.. :syntax/ state.next_id))
              (set slot.request id)
              (tset state.pending id
                    {: key : index :source slot.source :language slot.language})
              (table.insert fx
                            {:type :syntax/highlight
                             : id
                             :source slot.source
                             :language slot.language
                             :completion :syntax/completed})))

          (fn update-model [db model fx]
            (when (and model.id
                       (or (= model.kind :assistant) (= model.kind :thinking)
                           (= model.kind :user) (= model.kind :harness)))
              (local key (key-for model))
              (local source (source-for model))
              (local state db.syntax)
              (local old (. state.documents key))
              (when (or (not old) (not= old.source source))
                (local parsed (document key source))
                (tset dirty key true)
                (local next {: source
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
                    (local slot
                           (or (and same previous)
                               {:source block.text
                                :language block.language
                                :request (and previous previous.request)}))
                    (set slot.start block.source_start)
                    (table.insert next.slots slot)
                    (request state key index slot fx)))
                (tset state.documents key next))))

          {:fx [{:type :register/service
                 :name :syntax_projection
                 :value (fn [db model]
                          (local key (key-for model))
                          (local source (source-for model))
                          (local entry
                                 (and db.syntax (. db.syntax.documents key)))
                          (local cached (. documents key))
                          (when (and entry cached (= entry.source source))
                            (local captures {})
                            (each [_ slot (ipairs entry.slots)]
                              (local result
                                     (and slot.result
                                          (. cached.results slot.result)))
                              (when (and result (= result.source slot.source)
                                         (= result.language slot.language))
                                (tset captures slot.start result.data)))
                            {:document (and (= cached.source source)
                                            cached.document)
                             : captures
                             :revision entry.revision}))}
                {:type :register/event
                 :name :app/start
                 :handler (fn [db]
                            (set db.syntax
                                 {:next_id 0
                                  :epoch 0
                                  :pending {}
                                  :documents {}})
                            {: db})}
                {:type :register/event
                 :name :transcript/reset
                 :handler (fn [db]
                            (set db.syntax.pending {})
                            (set db.syntax.documents {})
                            (set db.syntax.epoch (+ db.syntax.epoch 1))
                            {: db})}
                {:type :register/interceptor
                 :value {:id :syntax/transcript
                         :before (fn [tx]
                                   ;; Release a reset cache only after its epoch was committed.
                                   (when (and tx.db.syntax
                                              (not= tx.db.syntax.epoch
                                                    cache-epoch))
                                     (each [key (pairs documents)]
                                       (tset documents key nil))
                                     (set cache-epoch tx.db.syntax.epoch))
                                   ;; Only changed document memos need pruning. At this boundary,
                                   ;; tx.db identifies the last accepted result IDs even after rollback.
                                   (each [key (pairs dirty)]
                                     (local cached (. documents key))
                                     (when cached
                                       (prune-results cached
                                                      (and tx.db.syntax
                                                           (. tx.db.syntax.documents
                                                              key))))
                                     (tset dirty key nil))
                                   (set tx.syntax_count
                                        (length (or (and tx.db.messages
                                                         tx.db.messages.blocks)
                                                    {})))
                                   tx)
                         :after (fn [tx]
                                  (when (and enabled
                                             tx.cofx.terminal.interactive
                                             tx.db.syntax tx.db.messages)
                                    (local blocks tx.db.messages.blocks)
                                    (for [index (+ tx.syntax_count 1) (length blocks)]
                                      (update-model tx.db (. blocks index)
                                                    tx.fx))
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
                                            (update-model tx.db model tx.fx))))))
                                  tx)}}
                {:type :register/event
                 :name :syntax/completed
                 :handler (fn [db event]
                            (local state db.syntax)
                            (local pending (. state.pending event.id))
                            (local fx {})
                            (when pending
                              (tset state.pending event.id nil)
                              (local entry (. state.documents pending.key))
                              (local slot
                                     (and entry (. entry.slots pending.index)))
                              (when (and slot (= slot.request event.id))
                                (set slot.request nil)
                                (if (and (= slot.source pending.source)
                                         (= slot.language pending.language))
                                    (do
                                      (set slot.done true)
                                      (when (and event.ok
                                                 (= (type event.data) :table))
                                        (local cached
                                               (assert (. documents pending.key)))
                                        (tset dirty pending.key true)
                                        ;; Memo data is owned once; db only accepts its result ID.
                                        ;; An aborted transaction cannot expose unaccepted captures.
                                        (tset cached.results event.id
                                              {:source pending.source
                                               :language pending.language
                                               :data (misa.snapshot event.data)})
                                        (set slot.result event.id)
                                        (set entry.revision
                                             (+ entry.revision 1))
                                        (table.insert fx
                                                      {:type :dispatch
                                                       :event {:type :ui/redraw}})))
                                    (request state pending.key pending.index
                                             slot fx))))
                            {: db : fx})}]})}
