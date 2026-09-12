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

(fn state [db]
  "Return the transcript state without performing layout."
  {:verbose (= (. (or db.messages {}) :verbose) true)})

;; ---------------------------------------------------------------------------
;; Per-item geometry
;;
;; An item's rendered view is a pure function of the item's model and the render
;; context, and a model is a pure function of one block plus the inputs the
;; projection already declares. So the view is retained keyed by the immutable
;; block:a replaced block replaces its entry, and an abandoned block drops one
;; with the weak key. Group chrome is keyed by its owner the same way.
;;
;; The retained entry is handed back to `misa.components.entry`, the per-item step
;; of the component collection, which rebuilds only what its own inputs changed.
;; The transcript therefore never re-renders a block that did not change, and it
;; never walks the whole collection to find that out.

(local block-geometry (setmetatable {} {:__mode :k}))
(local owner-geometry (setmetatable {} {:__mode :k}))

(fn slot [key]
  (or (. block-geometry key) (let [fresh {}] (tset block-geometry key fresh)
                               fresh)))

(fn owner-slot [owner part]
  (let [slots (or (. owner-geometry owner)
                  (let [fresh {}] (tset owner-geometry owner fresh) fresh))]
    (or (. slots part) (let [fresh {}] (tset slots part fresh) fresh))))

(fn item-entry [db role model context retained]
  "Render one item's view, reusing RETAINED when its inputs still apply."
  (let [previous (. retained :entry)
        entry (misa.components.entry db role model context previous
                                     (misa.themes.lookup db))]
    (tset retained :entry entry)
    entry))

(fn entry-lines [entry]
  (or (and entry entry.view entry.view.lines) []))

(fn attachment-lines [db model context]
  (if (and misa.attachments misa.attachments.lines model.attachments)
      (misa.attachments.lines db model.attachments context)
      []))

(fn model-for [db syntax-projections state block]
  "Copy one block into the model its component renders, with selection and syntax."
  (let [model {}]
    (each [key value (pairs block)] (tset model key value))
    (when model.chunks (set model.text (table.concat model.chunks)))
    (let [selected (and misa.selection misa.selection.state
                        (misa.selection.state db (selection-id model)))
          selecting (and selected (= selected.id (selection-id model)))]
      (when (and selecting model.text)
        (set model.text selected.text)
        ;; The presentation source is frozen. Live transport chunks must not
        ;; override it during syntax lookup.
        (set model.chunks nil))
      (when (and syntax-projections misa.syntax misa.syntax.for-model)
        (let [syntax (misa.syntax.for-model syntax-projections model)]
          (when syntax (set model.syntax syntax))))
      (values model selected))))

(fn group-models-for [db state]
  "Share one group model between the header and footer of each turn."
  (let [group-models {}
        turn-owners {}]
    (var turn nil)
    (each [_ owner (ipairs state.responses)]
      (if (not= owner.role :assistant) (set turn nil)
          (let [part (response-group db owner)
                project (or (and misa.costs misa.costs.group)
                            (and misa.costs misa.costs.response))
                cost (and project (project db owner.id))]
            (when (not turn)
              (set turn owner)
              (tset group-models turn.id
                    {:label :Assistant :started_wall_ms owner.started_wall_ms}))
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
                   (when (= owner turn) part.tokens_per_second))
              (when cost
                (let [total model.cost]
                  (set model.cost
                       (if (not total)
                           cost
                           {:type :money
                            :currency :USD
                            :amount (when (not (or total.pending cost.pending))
                                      (+ (or total.amount 0) (or cost.amount 0)))
                            :pending (or total.pending cost.pending false)
                            :unknown (or total.unknown cost.unknown false)
                            :estimated (or total.estimated cost.estimated false)}))))))))
    (values group-models turn-owners)))

(fn emit [frame item lines attachments]
  "Append one item, its rows, and the row its first row occupies.

`offsets` records the item's first row, which is its spacing row when one
precedes it, so a consumer can materialize any range without re-deriving it."
  (let [height (+ (length lines) (length attachments))
        space (and frame.pushed (> height 0) (not frame.document_id))]
    (tset frame.offsets (+ (length frame.items) 1) frame.row)
    (when space (set frame.row (+ frame.row 1)))
    (let [entry {:id item.id
                 :role item.role
                 :model item.model
                 :chrome item.chrome
                 :entry item.entry
                 : lines
                 : attachments
                 : height
                 :space_before space}]
      (when (> height 0) (set frame.pushed true))
      (table.insert frame.items entry)
      (when (and item.selected (not frame.selected_row))
        (set frame.selected_row (+ frame.row (if space 1 0))))
      (set frame.row (+ frame.row height)))))

(fn group-item [db frame owner part group-models]
  "Append one turn header or footer, reusing its rendered view."
  (when (and owner (= owner.role :assistant) (not frame.document_id))
    (when (not (. group-models owner.id))
      (let [model (response-group db owner)]
        (set model.label :Assistant)
        (let [project (or (and misa.costs misa.costs.group)
                          (and misa.costs misa.costs.response))]
          (when project (set model.cost (project db owner.id)))
          (tset group-models owner.id model))))
    (let [id (.. "response:" owner.id ":" part)
          role (.. :transcript.group_ part)
          model (. group-models owner.id)
          entry (item-entry db role model frame.context (owner-slot owner part))]
      (emit frame {: id : role : model :chrome true : entry}
            (entry-lines entry) []))))

(fn measure [db markdown presenters context]
  "Describe the transcript as ordered items with retained per-item geometry.

Each item carries its rendered rows, the attachments that follow them, its row
height, and whether a spacing row precedes it. `offsets` records the first row of
each item, so a consumer can materialize any row range without walking the rest.
Only blocks whose model or view inputs changed render again."
  (let [state (assert db.messages "message state is not initialized")
        document-id (and context context.document_id)
        render-context {:columns context.columns
                        :images context.images
                        :interactive context.interactive
                        : markdown}
        syntax-projections (and misa.syntax misa.syntax.all
                                (misa.syntax.all db))
        focused (and misa.selection misa.selection.state
                     (misa.selection.state db))
        blocks (if document-id
                   (icollect [_ block (ipairs state.blocks)]
                     (when (= (selection-id block) document-id) block))
                   state.blocks)
        (group-models turn-owners) (group-models-for db state)
        frame {:items []
               :offsets []
               :row 1
               :pushed false
               :selected_row nil
               :document_id document-id
               :context render-context}]
    (var previous-owner nil)
    (each [_ block (ipairs blocks)]
      (let [(model selected) (model-for db syntax-projections state block)
            owner (or (. turn-owners model.response_id)
                      (response db model.response_id))]
        (when (not= previous-owner owner)
          (group-item db frame previous-owner :footer group-models)
          (group-item db frame owner :header group-models)
          (set previous-owner owner))
        (let [projector (. presenters model.kind)
              presentation (and projector (projector model state selected))
              role (and presentation presentation.role)]
          (when role
            (each [key value (pairs (or presentation.model {}))]
              (tset model key value))
            (let [id (selection-id model)
                  entry (item-entry db role model render-context (slot block))
                  attachments (attachment-lines db model render-context)]
              (emit frame
                    {: id
                     : role
                     : model
                     : block
                     : entry
                     :selected (and focused selected (= selected.id focused.id))}
                    (entry-lines entry) attachments))))))
    (group-item db frame previous-owner :footer group-models)
    ;; A trailing spacing row closes a transcript that ends in a turn footer.
    (let [last-item (. frame.items (length frame.items))]
      (when (and (not document-id) last-item (> (or last-item.height 0) 0)
                 (= last-item.role :transcript.group_footer))
        (set (frame.row frame.trailing_space) (values (+ frame.row 1) true))))
    {:items frame.items
     :offsets frame.offsets
     :total (- frame.row 1)
     :context render-context
     :document_id document-id
     :trailing_space frame.trailing_space
     :selected_row frame.selected_row}))

(fn item-rows [db item context decorated?]
  "The rows one item contributes: its lines, then its attachments."
  (let [lines item.lines
        decorated (if (and decorated? misa.selection misa.selection.decorate
                           (not item.chrome))
                      (misa.selection.decorate db item.id
                                               (or (. item.model :text)
                                                   (. item.model :result)
                                                   (. item.model :argument_text)
                                                   "")
                                               lines)
                      lines)]
    (values decorated (or item.attachments []))))

(fn rows [db layout first count]
  "Materialize the rows in `[first, first+count)` from measured geometry.

Only the items covering that range are built into terminal rows, so a frame pays
for the visible rows rather than the whole transcript."
  (assert (and layout (= (type layout.items) :table))
          "transcript rows require a measured layout")
  (let [result []
        items layout.items
        offsets layout.offsets
        total (or layout.total 0)
        first (math.max 1 (math.floor (or first 1)))
        last (math.min total (- (+ first (math.max 0 (math.floor (or count 0))))
                                1))]
    (when (and (<= first last) (> (length items) 0))
      ;; Locate the first item whose rows reach `first`.
      (var (low high index) (values 1 (length items) nil))
      (while (and (<= low high) (not index))
        (let [middle (math.floor (/ (+ low high) 2))
              start (. offsets middle)
              content (+ start (if (. items middle :space_before) 1 0))
              stop (+ content (or (. items middle :height) 0) -1)]
          (if (< stop first) (set low (+ middle 1))
              (< content first) (set index middle)
              (set high (- middle 1)))))
      (var at (or index 1))
      (while (and (<= at (length items)) (<= (. offsets at) last))
        (let [item (. items at)
              start (. offsets at)
              content (+ start (if item.space_before 1 0))
              stop (- (+ content (or item.height 0)) 1)]
          ;; Items entirely before the window are skipped without building rows.
          (when (>= stop first)
            (when (and item.space_before (>= start first) (<= start last))
              (table.insert result
                            {:spans []
                             :transcript_id item.id
                             :source_part :spacing}))
            (when (> (or item.height 0) 0)
              (let [(lines attachments) (item-rows db item layout.context
                                                   (not layout.document_id))
                    height (or item.height 0)]
                (var offset 0)
                (each [_ line (ipairs lines)]
                  (let [position (+ content offset)]
                    (when (and (>= position first) (<= position last))
                      (table.insert result
                                    (owned-line line item.id
                                                (when item.chrome :chrome)))))
                  (set offset (+ offset 1)))
                (each [_ line (ipairs attachments)]
                  (let [position (+ content offset)]
                    (when (and (>= position first) (<= position last))
                      (table.insert result
                                    (owned-line line item.id :attachments))))
                  (set offset (+ offset 1)))
                (assert (= offset height)
                        "measured item height disagrees with rows"))))
          (set at (+ at 1))))
      ;; The closing spacing row after a final turn footer.
      (when (and layout.trailing_space (>= total first) (<= total last))
        (let [last-item (. items (length items))]
          (table.insert result
                        {:spans []
                         :transcript_id (or (and last-item last-item.id)
                                            :spacing)
                         :source_part :spacing}))))
    result))

(fn project [markdown presenters db context]
  "Render the complete transcript as terminal rows."
  (let [layout (misa.transcript.layout db context)]
    (rows db layout 1 layout.total)))

(fn document-layout [db document terminal]
  "Project the terminal rows belonging to one selection document."
  (icollect [_ line (ipairs (misa.transcript.project db
                                                     {:columns terminal.columns
                                                      :images terminal.images
                                                      :interactive true
                                                      :document_id document.id}))]
    (when (= line.transcript_id document.id) line)))

{: documents
 : document-layout
 : measure
 : presentations
 : project
 : rows
 : state}
