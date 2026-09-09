(local definitions (require :misa.definitions))

;; Highlighting is requested by transcript changes, never by rendering.

(fn key-for [model]
  (let [owner (tostring (or model.response_id ""))]
    (.. (length owner) ":" owner (tostring model.id))))

(fn tool-code [model]
  (when (and (= model.kind :tool_call) misa.tools misa.tools.presentation)
    (var code nil)
    (each [_ view (ipairs (or (. (misa.tools.presentation model) :arguments) []))
           &until code]
      (when (and (= view.role :content.code) view.model.language)
        (set code view.model)))
    code))

(fn source-for [model]
  (let [code (tool-code model)]
    (or (and code code.text) (and model.chunks (table.concat model.chunks))
        model.text "")))

(fn request [sequence key index slot]
  (if (or slot.request slot.done) {:next_id sequence : slot}
      (let [next-id (+ sequence 1)
            id (.. :syntax/ next-id)]
        {:next_id next-id
         :slot (misa.patch slot {:request id})
         :pending {id {: key
                       : index
                       :source slot.source
                       :language slot.language}}
         :fx [{:type :syntax/highlight
               : id
               :source slot.source
               :language slot.language
               :completion :syntax/completed}]})))

(fn model-update [state model]
  (var result nil)
  (when (and model.id (or (= model.kind :assistant) (= model.kind :thinking)
                          (= model.kind :user) (= model.kind :harness)
                          (tool-code model)))
    (let [key (key-for model)
          source (source-for model)
          old (. state.documents key)]
      (when (or (not old) (not= old.source source))
        (var next-id state.next_id)
        (let [pending {}
              effects []
              code (tool-code model)
              parsed (if code
                         {:blocks [{:kind :code_block
                                    :text source
                                    :language code.language
                                    :source_start 0
                                    :content_start 0
                                    :source_end (length source)}]}
                         (misa.markdown.parse source (and old old.document)))
              next {: source
                    :document parsed
                    :revision (+ (or (and old old.revision) 0) 1)
                    :slots {}}]
          (each [_ block (ipairs parsed.blocks)]
            (when (and (= block.kind :code_block) (not= block.language "")
                       (<= (length block.language) 64) (> (length block.text) 0)
                       (<= (length block.text) 1048576))
              (let [index (+ (length next.slots) 1)
                    previous (and old (. old.slots index))
                    same (and previous (= previous.source block.text)
                              (= previous.language block.language))
                    previous-slot (or (and same previous)
                                      {:source block.text
                                       :language block.language
                                       :request (and previous previous.request)})
                    requested (request next-id key index
                                       (misa.patch previous-slot
                                                   {:start block.source_start}))]
                (set next-id requested.next_id)
                (each [id item (pairs (or requested.pending {}))]
                  (tset pending id item))
                (each [_ effect (ipairs (or requested.fx []))]
                  (table.insert effects effect))
                (table.insert next.slots requested.slot))))
          (set result {:patch {:next_id next-id
                               : pending
                               :documents {key (misa.replace next)}}
                       :fx effects})))))
  result)

(fn update-models [state models]
  (var current state)
  (let [patch {:documents {} :pending {}}
        fx []]
    (each [_ model (ipairs models)]
      (let [update (model-update current model)]
        (when update
          (set current (misa.patch current update.patch))
          (set patch.next_id update.patch.next_id)
          (each [key value (pairs update.patch.documents)]
            (tset patch.documents key value))
          (each [id value (pairs update.patch.pending)]
            (tset patch.pending id value))
          (each [_ effect (ipairs update.fx)] (table.insert fx effect)))))
    (when (not= current state) {:patch {:syntax patch} : fx})))

(fn syntax-projections-value [inputs _ previous]
  (let [result {}]
    (each [key entry (pairs (or (. inputs 1) {}))]
      (let [old (and previous (. previous key))]
        (if (and old (= entry old.input))
            (tset result key old)
            (let [captures {}]
              (each [_ slot (ipairs entry.slots)]
                (when slot.data
                  (tset captures slot.start slot.data)))
              (tset result key
                    {:input entry
                     :value {:document entry.document
                             : captures
                             :source entry.source
                             :revision entry.revision}})))))
    result))

(fn syntax-for-model [projections model]
  "Return syntax captures for a presentation model."
  (let [key (key-for model)
        source (source-for model)
        entry (. projections key)
        projection (and entry entry.value)]
    (when (and projection (= projection.source source))
      projection)))

(fn initialize [db]
  {:patch {:syntax (misa.replace {:next_id 0 :pending {} :documents {}})}})

(fn transcript-updated [enabled update-models db event cofx]
  (when (and enabled cofx.terminal.interactive db.syntax misa.transcript
             misa.transcript.blocks)
    (update-models db.syntax
                   (misa.transcript.blocks db event.response_id event.block_id))))

(fn syntax-completed [request db event]
  (let [pending (. db.syntax.pending event.id)]
    (when pending
      (let [patch {:pending {event.id misa.delete}}
            entry (. db.syntax.documents pending.key)
            previous (and entry (. entry.slots pending.index))
            fx {}]
        (when (and previous (= previous.request event.id))
          (var slot (misa.patch previous {:request misa.delete}))
          (var revision entry.revision)
          (if (and (= slot.source pending.source)
                   (= slot.language pending.language))
              (do
                (set slot (misa.patch slot {:done true}))
                (when (and event.ok (= (type event.data) :table))
                  ;; Patch validation owns the capture payload; no external
                  ;; cache may change the meaning of a retained state.
                  (set slot (misa.patch slot {:data (misa.replace event.data)}))
                  (set revision (+ revision 1))
                  (table.insert fx {:type :dispatch :event {:type :ui/redraw}})))
              (let [requested (request db.syntax.next_id pending.key
                                       pending.index slot)]
                (set patch.next_id requested.next_id)
                (each [id item (pairs (or requested.pending {}))]
                  (tset patch.pending id item))
                (each [_ effect (ipairs (or requested.fx []))]
                  (table.insert fx effect))
                (set slot requested.slot)))
          (let [slots (icollect [index value (ipairs entry.slots)]
                        (if (= index pending.index)
                            slot
                            value))]
            (set patch.documents
                 {pending.key {:slots (misa.replace slots) : revision}})))
        {:patch {:syntax patch} : fx}))))

(fn build [context]
  "Declare transcript syntax highlighting."
  (let [enabled (not= (. (or (. (or context.config {}) :messages) {}) :markdown)
                      false)]
    (definitions.build :syntax
      [(let [definition {:id :syntax/projections
                         :inputs [[:db/path :syntax :documents]]
                         :compute syntax-projections-value}]
         {:catalog :subscriptions :id (. definition :id) :value definition})
       (let [definition {:id :syntax/projection
                         :inputs [[:syntax/projections]]
                         :compute (fn [inputs query]
                                    (let [entry (. (. inputs 1) (. query 2))]
                                      (and entry entry.value)))}]
         {:catalog :subscriptions :id (. definition :id) :value definition})
       {:catalog :services
        :id :syntax.all
        :value (fn [db]
                 "Return available syntax captures indexed by source identity."
                 (misa.sub db [:syntax/projections]))}
       {:catalog :services :id :syntax.for-model :value syntax-for-model}
       {:catalog :events :value {:event :app/start :handler initialize}}
       {:catalog :events
        :value {:event :transcript/reset
                :handler (fn [db]
                           {:patch {:syntax {:pending (misa.replace {})
                                             :documents (misa.replace {})}}})}}
       {:catalog :events
        :value {:event :transcript/updated
                :handler (fn [db event cofx]
                           (transcript-updated enabled update-models db event
                                               cofx))}}
       {:catalog :events
        :value {:event :syntax/completed
                :handler (fn [db event] (syntax-completed request db event))}}]
      {})))

{:build build}
