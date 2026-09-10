(local {: describe : response : response-group : selection-id}
       (require :misa.transcript.model))

(local owned-rows (setmetatable {} {:__mode :k}))

(fn owned-line [line id part]
  (let [previous (. owned-rows line)
        source-part (or part line.source_part)]
    (if (and previous (= previous.transcript_id id)
             (= previous.source_part source-part))
        previous
        (let [row (collect [key value (pairs line)] key value)]
          (set row.transcript_id id)
          (set row.source_part source-part)
          (tset owned-rows line row)
          row))))

(fn present-user [] {:role :transcript.user :model {:rail :rail.user}})

(fn present-assistant []
  {:role :transcript.assistant :model {:rail :rail.assistant}})

(fn present-thinking [model state selected]
  {:role (if (or state.verbose model.streaming selected) :transcript.thinking
             :transcript.thinking_collapsed)
   :model {:rail :rail.thinking}})

(fn present-tool-call [model state selected]
  (let [result-selected (and selected
                             (if selected.source_part
                                 (= selected.source_part :result)
                                 (not= model.result nil)))]
    {:role :transcript.tool_call
     :model {:rail (if model.is_error :rail.error :rail.tool)
             :collapsed (not (or state.verbose selected))
             :selection_text (when selected selected.text)
             :selection_source (when selected
                                 (if result-selected :result :args))}}))

(fn present-tool-result [model state selected]
  {:role :transcript.tool_result
   :model {:rail (if model.is_error :rail.error :rail.tool)
           :collapsed (not (or state.verbose selected))
           :selection_source (when selected :result)
           :selection_text (when selected selected.text)}})

(fn present-harness [model]
  {:role :transcript.harness
   :model {:rail (if (= model.level :error) :rail.error :rail.harness)}})

(local presentations {:user present-user
                      :assistant present-assistant
                      :thinking present-thinking
                      :tool_call present-tool-call
                      :tool_result present-tool-result
                      :harness present-harness})

(fn documents [db]
  "Describe selectable transcript sources independently of terminal layout."
  (let [result {}]
    (each [_ block (ipairs (or (. (or db.messages {}) :blocks) {}))]
      (var text (if (= block.kind :tool_call)
                    (or block.result block.argument_text
                        (and block.argument_chunks
                             (table.concat block.argument_chunks)))
                    (or block.text
                        (and block.chunks (table.concat block.chunks))
                        block.result block.argument_text)))
      (when (and (or (not text) (= text "")) block.arguments)
        (set text (describe block.arguments 0)))
      (when (and text (not= text ""))
        (let [document (misa.selection.document (selection-id block)
                                                (.. block.kind " · "
                                                    (table.concat (icollect [_ value (ipairs (misa.values.render {:type :timestamp
                                                                                                                  :value (or block.started_wall_ms
                                                                                                                             0)}))]
                                                                    value.text))
                                                    " — "
                                                    (misa.layout.clip (or (text:match "[^\r
]+")
                                                                          "")
                                                                      40))
                                                text)]
          (set document.source_part
               (if (= block.kind :tool_result) :result
                   (= block.kind :tool_call) (if (not= block.result nil)
                                                 :result
                                                 :args)
                   :body))
          (table.insert result document))))
    result))

(fn document-layout [db document terminal]
  "Project the terminal rows belonging to one selection document."
  (icollect [_ line (ipairs (misa.transcript.project db
                                                     {:columns terminal.columns
                                                      :images terminal.images
                                                      :interactive true
                                                      :document_id document.id}))]
    (when (= line.transcript_id document.id)
      line)))

(fn state [db]
  "Return the transcript state without performing layout."
  {:verbose (= (. (or db.messages {}) :verbose) true)})

(fn project [markdown presenters db render-context]
  "Render transcript blocks while reusing unchanged component geometry."
  (let [context-copy {}]
    (each [key value (pairs (or render-context {}))]
      (tset context-copy key value))
    (set context-copy.markdown markdown)
    (let [document-id context-copy.document_id]
      (set context-copy.document_id nil)
      (let [syntax-projections (and misa.syntax misa.syntax.all
                                    (misa.syntax.all db))
            (items state) (values []
                                  (assert db.messages
                                          "message state is not initialized"))
            blocks (if document-id
                       (icollect [_ block (ipairs state.blocks)]
                         (when (= (selection-id block) document-id)
                           block))
                       state.blocks)]
        (var previous-owner nil)
        (let [group-models {}
              turn-owners {}]
          (var turn nil)
          (each [_ owner (ipairs state.responses)]
            (if (not= owner.role :assistant)
                (set turn nil)
                (let [part (response-group db owner)
                      project (or (and misa.costs misa.costs.group)
                                  (and misa.costs misa.costs.response))
                      cost (and project (project db owner.id))]
                  (when (not turn)
                    (set turn owner)
                    (tset group-models turn.id
                          {:label :Assistant
                           :started_wall_ms owner.started_wall_ms}))
                  (tset turn-owners owner.id turn)
                  (let [model (. group-models turn.id)]
                    (when part.elapsed_ms
                      (set model.elapsed_ms
                           (math.max (or model.elapsed_ms 0)
                                     (+ (- owner.started_monotonic_ms
                                           turn.started_monotonic_ms)
                                        part.elapsed_ms))))
                    (when (not (or (= model.status :running)
                                   (= model.status :streaming)))
                      (set model.status part.status))
                    ;; A per-request rate would misrepresent a multi-request turn.
                    (set model.tokens_per_second
                         (when (= owner turn)
                           part.tokens_per_second))
                    (when cost
                      (let [total model.cost]
                        (set model.cost
                             (if (not total)
                                 cost
                                 {:type :money
                                  :currency :USD
                                  :amount (when (not (or total.pending
                                                         cost.pending))
                                            (+ (or total.amount 0)
                                               (or cost.amount 0)))
                                  :pending (or total.pending cost.pending false)
                                  :unknown (or total.unknown cost.unknown false)
                                  :estimated (or total.estimated cost.estimated
                                                 false)}))))))))

          (fn group-item [owner part]
            (when (and owner (= owner.role :assistant) (not document-id))
              (when (not (. group-models owner.id))
                (let [model (response-group db owner)]
                  (set model.label :Assistant)
                  (let [project (or (and misa.costs misa.costs.group)
                                    (and misa.costs misa.costs.response))]
                    (when project
                      (set model.cost (project db owner.id)))
                    (tset group-models owner.id model))))
              (table.insert items
                            {:id (.. "response:" owner.id ":" part)
                             :role (.. :transcript.group_ part)
                             :chrome true
                             :model (. group-models owner.id)})))

          (each [_ source (ipairs blocks)]
            (let [model {}]
              (each [key value (pairs source)]
                (tset model key value))
              (when model.chunks
                (set model.text (table.concat model.chunks)))
              (let [selected (and misa.selection misa.selection.state
                                  (misa.selection.state db (selection-id model)))
                    selecting (and selected
                                   (= selected.id (selection-id model)))]
                (when (and selecting model.text)
                  (set model.text selected.text)
                  ;; The presentation source is frozen. Live transport
                  ;; chunks must not override it during syntax lookup.
                  (set model.chunks nil))
                (when (and syntax-projections misa.syntax misa.syntax.for-model)
                  (let [syntax (misa.syntax.for-model syntax-projections model)]
                    (when syntax
                      (set model.syntax syntax))))
                (let [owner (or (. turn-owners model.response_id)
                                (response db model.response_id))]
                  (when (not= previous-owner owner)
                    (group-item previous-owner :footer)
                    (group-item owner :header)
                    (set previous-owner owner))
                  (let [projector (. presenters model.kind)]
                    (let [presentation (and projector
                                            (projector model state
                                                       (and selecting selected)))
                          role (and presentation presentation.role)]
                      (each [key value (pairs (or (and presentation
                                                       presentation.model)
                                                  {}))]
                        (tset model key value))
                      (when role
                        (table.insert items
                                      {:id (selection-id model) : role : model}))))))))
          (group-item previous-owner :footer)
          (let [projection (misa.components.project db
                                                    (if document-id
                                                        :selection_geometry
                                                        :transcript)
                                                    items context-copy)
                result []]
            (each [index item (ipairs items)]
              (let [model item.model
                    rendered (. projection.views index)
                    lines (if (and misa.selection misa.selection.decorate
                                   (not document-id) (not item.chrome))
                              (misa.selection.decorate db (selection-id model)
                                                       (or model.text
                                                           model.result
                                                           model.argument_text
                                                           "")
                                                       (or rendered.lines []))
                              (or rendered.lines []))]
                (when (and (> (length result) 0) (> (length lines) 0)
                           (not document-id))
                  (table.insert result
                                {:spans []
                                 :transcript_id item.id
                                 :source_part :spacing}))
                (each [_ line (ipairs lines)]
                  (table.insert result
                                (owned-line line item.id
                                            (when item.chrome :chrome))))
                (when (and (= item.role :transcript.group_footer)
                           (= index (length items)) (> (length lines) 0)
                           (not document-id))
                  (table.insert result
                                {:spans []
                                 :transcript_id item.id
                                 :source_part :spacing}))
                (when (and misa.attachments misa.attachments.lines
                           model.attachments)
                  (each [_ line (ipairs (misa.attachments.lines db
                                                                model.attachments
                                                                context-copy))]
                    (tset result (+ (length result) 1)
                          (owned-line line item.id :attachments))))))
            result))))))

{: documents : document-layout : project : state : presentations}
