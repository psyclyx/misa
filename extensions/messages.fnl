;; Canonical transcript models and projection. Interactive output is composed

;; only from db.messages.blocks by the root managed view.

(fn lower [value] (: (tostring value) :lower))

;; Transcript values cross the final trust boundary here. Keep line feeds, turn

;; tabs/CR into stable text, strip terminal controls, and repair malformed UTF-8

;; before any component can project the value into a semantic view.

(fn printable-text [value]
  (var (out i ___length___) (values {} 1 (length value)))

  (fn continuation [byte]
    (and (and byte (>= byte 128)) (<= byte 191)))

  (while (<= i ___length___)
    (local byte (value:byte i))
    (if (= byte 27) (let [next-byte (value:byte (+ i 1))]
                      (if (= next-byte 91)
                          (do
                            (set i (+ i 2))
                            (while (<= i ___length___)
                              (local current (value:byte i))
                              (set i (+ i 1))
                              (when (and (>= current 64) (<= current 126))
                                (lua :break))))
                          (= next-byte 93)
                          (do
                            (set i (+ i 2))
                            (while (<= i ___length___)
                              (local current (value:byte i))
                              (when (= current 7) (set i (+ i 1)) (lua :break))
                              (when (and (= current 27)
                                         (= (value:byte (+ i 1)) 92))
                                (set i (+ i 2))
                                (lua :break))
                              (set i (+ i 1))))
                          (set i (+ i 1)))) (= byte 9)
        (do
          (tset out (+ (length out) 1) " ")
          (set i (+ i 1))) (= byte 13)
        (do
          (tset out (+ (length out) 1) "\n")
          (set i (+ i 1))
          (when (= (value:byte i) 10) (set i (+ i 1)))) (= byte 10)
        (do
          (tset out (+ (length out) 1) "\n")
          (set i (+ i 1))) (or (< byte 32) (= byte 127)) (set i (+ i 1))
        (let [width (or (and (< byte 128) 1)
                        (or (and (and (>= byte 194) (<= byte 223)) 2)
                            (or (and (and (>= byte 224) (<= byte 239)) 3)
                                (or (and (and (>= byte 240) (<= byte 244)) 4) 0))))
              (b2 b3 b4) (values (value:byte (+ i 1)) (value:byte (+ i 2))
                                 (value:byte (+ i 3)))
              valid (or (or (or (= width 1) (and (= width 2) (continuation b2)))
                            (and (and (and (and (= width 3) (continuation b2))
                                           (continuation b3))
                                      (not (and (= byte 224) (< b2 160))))
                                 (not (and (= byte 237) (> b2 159)))))
                        (and (and (and (and (and (= width 4) (continuation b2))
                                            (continuation b3))
                                       (continuation b4))
                                  (not (and (= byte 240) (< b2 144))))
                             (not (and (= byte 244) (> b2 143)))))]
          (if valid (do
                      (when (not (and (and (= byte 194) (>= b2 128))
                                      (<= b2 159)))
                        (tset out (+ (length out) 1)
                              (value:sub i (- (+ i width) 1))))
                      (set i (+ i width)))
              (do
                (tset out (+ (length out) 1) "�")
                (set i (+ i 1)))))))
  (table.concat out))

(fn truncate-text [value limit]
  (if (<= (length value) limit) value
      (do
        (var boundary limit)
        (while (and (and (and (> boundary 0) (value:byte (+ boundary 1)))
                         (>= (value:byte (+ boundary 1)) 128))
                    (<= (value:byte (+ boundary 1)) 191))
          (set boundary (- boundary 1)))
        (.. (value:sub 1 boundary) "… [truncated "
            (tostring (- (length value) boundary)) " bytes]"))))

(fn copy-structural [value policy depth key]
  (if (and key (. policy.redact (lower key)))
      "[redacted]"
      (let [kind (type value)]
        (if (= kind :string)
            (truncate-text (printable-text value) policy.max_string)
            (if (not= kind :table) value
                (if (>= depth policy.max_depth)
                    "[truncated structure]"
                    (do
                      (var (result count) (values {} 0))
                      (each [child-key child (pairs value)]
                        (set count (+ count 1))
                        (when (> count policy.max_items)
                          (tset result "…" "[truncated items]")
                          (lua :break))
                        (tset result child-key
                              (copy-structural child policy (+ depth 1)
                                               child-key)))
                      result)))))))

(fn describe [value depth]
  (let [kind (type value)]
    (if (= kind :string) (printable-text value)
        (if (not= kind :table) (printable-text (tostring value))
            (if (> depth 3)
                "{…}"
                (let [___values___ {}]
                  (each [key child (pairs value)]
                    (tset ___values___ (+ (length ___values___) 1)
                          (.. (printable-text (tostring key)) "="
                              (describe child (+ depth 1)))))
                  (table.sort ___values___)
                  (.. "{" (table.concat ___values___ ", ") "}")))))))

(fn noninteractive-commit [db role model cofx markdown]
  (if (or cofx.terminal.interactive (not misa.render_component)) {}
      (let [rendered (misa.render_component db role model
                                            {:columns cofx.terminal.columns
                                             :interactive false
                                             : markdown})]
        (if (= (length (or rendered.lines {})) 0) {}
            [{:lines rendered.lines :type :view/commit}]))))

(fn clock [cofx]
  (let [value (assert cofx.clock "native clock coeffect is missing")]
    (values value.wall_ms value.monotonic_ms)))

(fn appended [items value]
  (local next (icollect [_ item (ipairs items)] item))
  (table.insert next value)
  next)

(fn updated-fx [fx response-id block-id]
  (appended (or fx []) {:type :dispatch
                        :event {:type :transcript/updated :response_id response-id :block_id block-id}}))

(fn message-update [db fx]
  {:patch {:messages (misa.replace db.messages)} : fx})

(fn append-response [db id role cofx status timing]
  (local state (assert db.messages "message state is not initialized"))
  (assert (not (. state.by_response id)) (.. "duplicate transcript response: " (tostring id)))
  (local (wall mono) (clock cofx))
  (local owner {:block_count 0 :block_start (+ (length state.blocks) 1) : id : role
                :started_monotonic_ms (or (and timing timing.started_monotonic_ms) mono)
                :started_wall_ms (or (and timing timing.started_wall_ms) wall) :status (or status :streaming)})
  (values (misa.patch db {:messages {:responses (misa.replace (appended state.responses owner))
                                    :by_response {id (+ (length state.responses) 1)} :scroll 0}})
          owner))

(fn response [db id]
  (local state db.messages)
  (local index (and state state.by_response (. state.by_response id)))
  (and index (. state.responses index)))

(fn transcript-blocks [db response-id block-id]
  ;; Nil owner is an explicit full-transcript lookup for imports/fixture sync.
  ;; Scoped lookups allocate only the array; its records remain canonical.
  (local blocks (or (and db.messages db.messages.blocks) []))
  (if (= response-id nil) blocks
      (let [owner (response db response-id)
            result []]
        (when owner
          (for [index owner.block_start (- (+ owner.block_start owner.block_count) 1)]
            (local block (. blocks index))
            (when (and block (or (= block-id nil) (= block.id block-id)))
              (table.insert result block))))
        result)))

;; A visible response includes the executions caused by its generated calls.
;; Use the latest completion, not a sum: tools may run concurrently. Throughput
;; remains the provider request measurement, independent of execution latency.
(fn response-group [db owner]
  (var completed owner.completed_monotonic_ms)
  (var pending false)
  (each [_ block (ipairs (transcript-blocks db owner.id))]
    (when (= block.kind :tool_call)
      (when (or (= block.status :pending) (= block.status :running))
        (set pending true))
      (when block.execution_completed_monotonic_ms
        (set completed (math.max (or completed block.execution_completed_monotonic_ms)
                                  block.execution_completed_monotonic_ms)))))
  (local status (if (and (= owner.status :complete) pending) :running owner.status))
  {:started_wall_ms owner.started_wall_ms : status
   :elapsed_ms (when (and completed (not= status :streaming) (not= status :running))
                 (math.max 0 (- completed owner.started_monotonic_ms)))
   :tokens_per_second owner.tokens_per_second})

(fn append-block [db response-model block]
  (local owner (assert (response db response-model.id) "unknown transcript response"))
  (local next-block (misa.patch block {:response_id owner.id :role owner.role :started_wall_ms owner.started_wall_ms}))
  (local responses (icollect [_ previous (ipairs db.messages.responses)]
                     (if (= previous.id owner.id) (misa.patch previous {:block_count (+ owner.block_count 1)}) previous)))
  (values (misa.patch db {:messages {:blocks (misa.replace (appended db.messages.blocks next-block))
                                    :responses (misa.replace responses) :scroll 0}})
          next-block))

(fn find-block [db response-model id]
  (let [blocks db.messages.blocks]
    (for [index response-model.block_start (- (+ response-model.block_start
                                                 response-model.block_count)
                                              1)]
      (when (and (. blocks index) (= (. blocks index :id) id))
        (let [___antifnl_rtn_1___ (. blocks index)]
          (lua "return ___antifnl_rtn_1___"))))
    nil))

(fn find-tool-section [db call-id]
  (for [index (length db.messages.blocks) 1 (- 1)]
    (local block (. db.messages.blocks index))
    (when (and call-id (or (= block.kind :tool_call) (= block.kind :tool_result))
               (= block.call_id call-id))
      (lua "return block")))
  nil)

;; Structural tool previews have a size budget; conversation text does not.
(fn append-chunk [chunks text]
  (local next (icollect [_ chunk (ipairs (or chunks []))] chunk))
  (table.insert next text)
  next)

(fn text-delta [block event]
  (local text (printable-text (tostring (or event.text ""))))
  (when (not= text "")
    {:chunks (misa.replace (append-chunk block.chunks text))
     :byte_count (+ (or block.byte_count 0) (length text))}))

(fn tool-delta [previous event policy]
  (local replacement (not= event.arguments_json nil))
  (local block (if replacement
                  (misa.patch previous {:argument_chunks (misa.replace []) :argument_bytes 0
                                        :arguments_truncated misa.delete})
                  previous))
  (local raw (if replacement event.arguments_json event.arguments_json_delta))
  (local value (when (not= raw nil) (printable-text (tostring raw))))
  (local remaining (- policy.max_string (or block.argument_bytes 0)))
  (local active (and value (not block.arguments_truncated)))
  (local truncated (and active (or (<= remaining 0) (> (length value) remaining))))
  (local chunk (when active (if (<= remaining 0) "… [truncated]"
                               truncated (truncate-text value remaining) value)))
  {:name (when (not= event.name nil) (printable-text (tostring event.name)))
   :call_id (when (not= event.call_id nil) (printable-text (tostring event.call_id)))
   :argument_chunks (when (or active replacement)
                      (misa.replace (if active (append-chunk block.argument_chunks chunk) block.argument_chunks)))
   :argument_bytes (when (or active replacement)
                     (if truncated policy.max_string active (+ (or block.argument_bytes 0) (length value)) 0))
   :arguments_truncated (if truncated true replacement misa.delete nil)
   :arguments (when (not= event.arguments nil) (misa.replace (copy-structural event.arguments policy 0)))})

(fn replace-transcript-block [db block patch]
  (local replacement (misa.patch block patch))
  (local blocks (icollect [_ previous (ipairs db.messages.blocks)]
                 (if (= previous block) replacement previous)))
  {:patch {:messages {:blocks (misa.replace blocks)}}
   :fx (when (not= replacement block) (updated-fx nil block.response_id block.id))})

(fn finish-block [block event policy]
  (local value (or event {}))
  (local name (when (not= value.name nil) (printable-text (tostring value.name))))
  (local has-arguments (or (not= value.arguments nil) (not= block.arguments nil)))
  (misa.patch block
              {:description misa.delete
               :text (when block.chunks (table.concat block.chunks))
               :chunks misa.delete
               :arguments (when (not= value.arguments nil) (misa.replace (copy-structural value.arguments policy 0)))
               :name name
               :call_id (when (not= value.call_id nil) (printable-text (tostring value.call_id)))
               :argument_text (if has-arguments misa.delete block.argument_chunks (table.concat block.argument_chunks) nil)
               :argument_chunks misa.delete :argument_bytes misa.delete :streaming false}))

(fn finalized-response [db owner now interrupted policy]
  (local blocks
         (icollect [index block (ipairs db.messages.blocks)]
           (if (and (>= index owner.block_start) (< index (+ owner.block_start owner.block_count)))
               (let [finished (if (or interrupted block.streaming) (finish-block block nil policy) block)]
                 (if interrupted (misa.patch finished {:interrupted true}) finished))
               block)))
  (values blocks
          (misa.patch owner {:status (if interrupted :interrupted :complete)
                             :completed_monotonic_ms now :elapsed_ms (math.max 0 (- now owner.started_monotonic_ms))})))

(fn response-update [db owner blocks fx]
  (local responses (icollect [_ previous (ipairs db.messages.responses)]
                     (if (= previous.id owner.id) owner previous)))
  {:patch {:messages {:responses (misa.replace responses) :blocks (misa.replace blocks)
                      }}
   :fx (if (> owner.block_count 0) (updated-fx fx owner.id) fx)})

(fn selection-id [block]
  (let [owner (tostring (or block.response_id ""))]
    (.. (length owner) ":" owner (tostring block.id))))

(fn selection-key [selected]
  (when selected {:id selected.id :first selected.first :last selected.last}))

(fn same-selection [a b]
  (and a b (= a.id b.id) (= a.first b.first) (= a.last b.last)))

;; Anchors identify content rather than its absolute screen row. Keeping these
;; in state makes speculative projections and resize renders deterministic.
(fn line-anchors [lines]
  (var (previous offset) (values nil 0))
  (icollect [_ line (ipairs lines)]
    (let [id line.transcript_id
          part (or line.source_part :body)]
      (var (source last) nil)
      (each [_ span (ipairs (or line.spans []))]
        (when (and (or span.source span.selection_marker) span.source_start span.source_end)
          (set source (math.min (or source span.source_start) span.source_start))
          (set last (math.max (or last span.source_end) span.source_end))))
      (set source (or source line.source_start -1))
      (set offset (if (and previous (= previous.id id) (= previous.part part) (= previous.source source)) (+ offset 1) 0))
      (set previous {: id : part : source : last : offset})
      previous)))

(fn anchored-row [anchors anchor fallback]
  (var (found distance offset-distance) (values nil math.huge math.huge))
  (var (exact exact-offset) (values nil math.huge))
  (when anchor
    (each [index candidate (ipairs anchors)]
      (when (and (= candidate.id anchor.id) (or (not anchor.part) (= candidate.part anchor.part)))
        (local delta (if (and candidate.last (<= candidate.source anchor.source) (< anchor.source candidate.last))
                         0 (math.abs (- candidate.source anchor.source))))
        (local offset-delta (math.abs (- candidate.offset anchor.offset)))
        ;; After displacement, prefer the original source start if it still
        ;; exists. A containing range is only a fallback for a rewrapped row.
        (when (and (= candidate.source anchor.source) (< offset-delta exact-offset))
          (set (exact exact-offset) (values index offset-delta)))
        (when (or (< delta distance) (and (= delta distance) (< offset-delta offset-distance)))
          (set (found distance offset-distance) (values index delta offset-delta))))))
  (or exact found fallback))

(fn same-anchor [a b]
  (and a b (= a.id b.id) (= a.part b.part) (= a.source b.source)
       (= a.offset b.offset)))

(fn viewport-top [messages anchors bottom]
  (if (not messages.top) bottom
      ;; Scrolling owns a physical row. Only relocate its content when layout
      ;; changes have actually displaced that row; ordinary renders must not
      ;; reinterpret an explicit scroll as a request to find the block again.
      (or (not messages.anchor) (same-anchor (. anchors messages.top) messages.anchor)) messages.top
      (anchored-row anchors messages.anchor messages.top)))

(fn transcript-viewport [db context available-lines]
  (local room (math.max 0 (math.floor (or available-lines 0))))
  (if (= room 0) {:first 1 :room 0 :total 0 :lines []}
      (let [lines (misa.transcript_projection db context)
            selected (and misa.selection_projection (misa.selection_projection db))
            key (selection-key selected)
            bottom (math.max 1 (+ (- (length lines) room) 1))
            anchors (line-anchors lines)]
        (var first (math.max 1 (math.min (viewport-top db.messages anchors bottom) bottom)))
        (when (and selected (not (same-selection db.messages.scroll_selection key)))
          (each [index line (ipairs lines)]
            (when (and (or line.selected line.selection_anchor)
                       (or (not line.selection_id) (= line.selection_id selected.id)))
              (set first (math.max 1 (math.min first index)))
              (when (>= index (+ first room)) (set first (+ (- index room) 1)))
              (lua :break))))
        {: first : room :total (length lines) :selection key : anchors
         :lines (icollect [index (ipairs lines) &until (>= index (+ first room))]
                  (when (>= index first) (. lines index)))})))

(local presentations
       {:user (fn [] {:role :transcript.user :model {:rail :rail.user}})
        :assistant (fn [] {:role :transcript.assistant :model {:rail :rail.assistant}})
        :thinking (fn [model state selected]
                    {:role (if (or state.verbose model.streaming selected) :transcript.thinking :transcript.thinking_collapsed)
                     :model {:rail :rail.thinking}})
        :tool_call (fn [model state selected]
                     (local result-selected (and selected (if selected.source_part (= selected.source_part :result)
                                                            (not= model.result nil))))
                     {:role :transcript.tool_call
                      :model {:rail (if model.is_error :rail.error :rail.tool)
                              :collapsed (not (or state.verbose selected))
                              :selection_text (when selected selected.text)
                              :selection_source (when selected (if result-selected :result :args))}})
        :tool_result (fn [model state selected]
                       {:role :transcript.tool_result
                        :model {:rail (if model.is_error :rail.error :rail.tool) :collapsed (not (or state.verbose selected))
                                :selection_source (when selected :result)
                                :selection_text (when selected selected.text)}})
        :harness (fn [model]
                   {:role :transcript.harness :model {:rail (if (= model.level :error) :rail.error :rail.harness)}})})

{:setup (fn [context]
          (local setup-fx [])
          (local projectors {})
          (each [id project (pairs presentations)] (tset projectors id project))
          (table.insert setup-fx
                        {:type :register/setup-effect :name :register/transcript-presentation
                         :handler (fn [effect]
                                    (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                 (= (type effect.value) :function) (not (. projectors effect.id)))
                                            "invalid or duplicate transcript presentation")
                                    (tset projectors effect.id effect.value))})
          (local delta-handlers {:assistant text-delta :thinking text-delta :tool_call tool-delta})
          (table.insert setup-fx
                        {:type :register/setup-effect :name :register/transcript-delta
                         :handler (fn [effect]
                                    (assert (and (= (type effect.id) :string) (not= effect.id "")
                                                 (= (type effect.value) :function) (not (. delta-handlers effect.id)))
                                            "invalid or duplicate transcript delta handler")
                                    (tset delta-handlers effect.id effect.value))})
          (var config (or (and (= (type context.config) :table)
                               context.config.messages)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local markdown
                 (and (not= config.plain true) (not= config.markdown false)))
          (local policy {:max_depth (or config.max_depth 8)
                         :max_items (or config.max_items 64)
                         :max_string (or config.max_string 4000)
                         :redact {}})
          (assert (and (= (type policy.max_string) :number)
                       (> policy.max_string 0))
                  "messages.max_string must be positive")
          (each [_ key (ipairs (or config.redact_keys
                                   [:authorization
                                    :api_key
                                    :password
                                    :secret
                                    :token]))]
            (tset policy.redact (lower key) true))
          (table.insert setup-fx
                        {:type :register/keybinding
                         :value {:action :toggle_verbose
                                 :context :global
                                 :default [:alt+t]}})
          (table.insert setup-fx
                        {:type :register/keybinding
                         :value {:action :transcript_up
                                 :context :global
                                 :default [:page_up :alt+k]}})
          (table.insert setup-fx
                        {:type :register/keybinding
                         :value {:action :transcript_down
                                 :context :global
                                 :default [:page_down :alt+j]}})
          (when (misa.has_setup_effect :register/indicator)
            (table.insert setup-fx
                          {:type :register/indicator
                           :value {:hotkey {:action :toggle_verbose
                                            :context :global}
                                   :icon "≡"
                                   :id :transcript-detail
                                   :label :detail
                                   :query [:messages/detail-indicator]}}))
          (table.insert setup-fx
                        {:type :register/sub
                         :value {:id :messages/detail-indicator :inputs [[:db/path :messages :verbose]]
                                 :compute (fn [inputs]
                                            {:type :text :value (if (. inputs 1) :verbose :summary)})}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    {:patch {:messages (misa.replace
                                                         {:blocks [] :by_response {} :next_id 0 :responses []
                                                          :scroll 0 :verbose (= config.verbose true)})}})})
          (table.insert setup-fx
                        {:type :register/event :name :messages/toggle-verbose
                         :handler (fn [db]
                                    {:patch {:messages {:verbose (not db.messages.verbose) :scroll 0}}
                                     :fx [{:event {:type :ui/redraw} :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :messages/scroll
                         :handler (fn [db event cofx]
                                    (local terminal (or (and cofx cofx.terminal) {:lines 0 :columns 80}))
                                    (local viewport
                                           (if misa.ui_regions
                                               (accumulate [found nil _ region (ipairs (misa.ui_regions db terminal))]
                                                 (or found (when (= region.id :transcript) region.viewport)))
                                               (transcript-viewport db {:columns terminal.columns :images terminal.images :interactive true}
                                                                    terminal.lines)))
                                    (if (or (not viewport) (= viewport.room 0)) {:fx [{:type :terminal/read}]}
                                        (let [bottom (math.max 1 (+ (- viewport.total viewport.room) 1))
                                              first (math.max 1 (math.min bottom (- viewport.first event.delta)))]
                                          {:patch {:messages {:top (if (< first bottom) first misa.delete)
                                                               :anchor (if (< first bottom) (misa.replace (. viewport.anchors first)) misa.delete)
                                                               :scroll_selection (misa.replace viewport.selection)
                                                               :scroll (math.max 0 (- bottom first))}}
                                           :fx [{:type :terminal/read}]})))})
          (local scroll-inputs {:wheel_up (fn [] 3) :wheel_down (fn [] -3)
                                :transcript_up (fn [cofx] (math.max 1 (math.floor (/ cofx.terminal.lines 2))))
                                :transcript_down (fn [cofx] (- (math.max 1 (math.floor (/ cofx.terminal.lines 2)))))})
          (table.insert setup-fx
                        {:type :register/event-route
                         :value {:id :messages/global-keys :event :terminal/input :priority 400
                                 :context [:db/path]
                                 :resolve (fn [db event cofx]
                                            (when (and (not db.picker) (not db.dialog) misa.keybinding_action)
                                              (local action (misa.keybinding_action :global event))
                                              (local scroll (or (. scroll-inputs event.kind) (. scroll-inputs action)))
                                              (if scroll {:type :messages/scroll :delta (scroll cofx)}
                                                  (= action :toggle_verbose) {:type :messages/toggle-verbose})))}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:binding {:action :toggle_verbose
                                           :context :global}
                                 :event {:type :messages/toggle-verbose}
                                 :id :transcript.detail
                                 :label "Toggle transcript detail"}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:binding {:action :transcript_up
                                           :context :global}
                                 :event {:delta 10 :type :messages/scroll}
                                 :id :transcript.up
                                 :label "Scroll transcript up"}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:binding {:action :transcript_down
                                           :context :global}
                                 :event {:delta (- 10) :type :messages/scroll}
                                 :id :transcript.down
                                 :label "Scroll transcript down"}})
          (when (misa.has_setup_effect :register/selection-source)
            (table.insert setup-fx
                          {:type :register/selection-source
                           :id :transcript
                           :layout (fn [db document terminal]
                                     (icollect [_ line (ipairs (misa.transcript_projection db
                                                                {:columns terminal.columns :images terminal.images
                                                                 :interactive true :document_id document.id}))]
                                       (when (= line.transcript_id document.id) line)))
                           :value (fn [db]
                                    (local result {})
                                    (each [_ block (ipairs (or (. (or db.messages
                                                                      {})
                                                                  :blocks)
                                                               {}))]
                                      (var text
                                           (if (= block.kind :tool_call)
                                               (or block.result block.argument_text
                                                   (and block.argument_chunks (table.concat block.argument_chunks)))
                                               (or block.text (and block.chunks (table.concat block.chunks))
                                                   block.result block.argument_text)))
                                      (when (and (or (not text) (= text "")) block.arguments)
                                        (set text (describe block.arguments 0)))
                                      (when (and text (not= text ""))
                                        (local document (misa.selection_document (selection-id block)
                                                                       (.. block.kind
                                                                           " · "
                                                                           (table.concat
                                                                             (icollect [_ value (ipairs
                                                                               (misa.render_value {:type :timestamp
                                                                                                   :value (or block.started_wall_ms 0)}))]
                                                                               value.text))
                                                                           " — "
                                                                           (misa.layout.clip (or (text:match "[^\r
]+")
                                                                                                 "")
                                                                                             40))
                                                                       text))
                                        (set document.source_part (if (= block.kind :tool_result) :result
                                                                      (= block.kind :tool_call) (if (not= block.result nil) :result :args)
                                                                      :body))
                                        (table.insert result document)))
                                    result)}))
          (table.insert setup-fx
                        {:type :register/service
                         :name :messages_projection
                         :value (fn [db]
                                  {:verbose (= (. (or db.messages {}) :verbose)
                                               true)})})
          (table.insert setup-fx
                        {:type :register/service
                         :name :transcript_projection
                         :value (fn [db render-context]
                                  (local context-copy {})
                                  (each [key value (pairs (or render-context {}))]
                                    (tset context-copy key value))
                                  (set context-copy.markdown markdown)
                                  (local document-id context-copy.document_id)
                                  (set context-copy.document_id nil)
                                  (local syntax-projections (and misa.syntax_projections
                                                                 (misa.syntax_projections db)))
                                  (local (items state)
                                         (values []
                                                 (assert db.messages
                                                         "message state is not initialized")))
                                  (local blocks (if document-id
                                                    (icollect [_ block (ipairs state.blocks)]
                                                      (when (= (selection-id block) document-id) block))
                                                    state.blocks))
                                  (var previous-owner nil)
                                  (local group-models {})
                                  (local turn-owners {})
                                  (var turn nil)
                                  (each [_ owner (ipairs state.responses)]
                                    (if (not= owner.role :assistant) (set turn nil)
                                        (do
                                          (local part (response-group db owner))
                                          (local project (or misa.group_cost_projection misa.response_cost_projection))
                                          (local cost (and project (project db owner.id)))
                                          (when (not turn)
                                            (set turn owner)
                                            (tset group-models turn.id {:label :Assistant
                                                                       :started_wall_ms owner.started_wall_ms}))
                                          (tset turn-owners owner.id turn)
                                          (local model (. group-models turn.id))
                                          (when part.elapsed_ms
                                            (set model.elapsed_ms (math.max (or model.elapsed_ms 0)
                                              (+ (- owner.started_monotonic_ms turn.started_monotonic_ms) part.elapsed_ms))))
                                          (when (not (or (= model.status :running) (= model.status :streaming)))
                                            (set model.status part.status))
                                          ;; A per-request rate would misrepresent a multi-request turn.
                                          (set model.tokens_per_second (when (= owner turn) part.tokens_per_second))
                                          (when cost
                                            (local total model.cost)
                                            (set model.cost (if (not total) cost {:type :money :currency :USD
                                                             :amount (when (not (or total.pending cost.pending))
                                                                       (+ (or total.amount 0) (or cost.amount 0)))
                                                             :pending (or total.pending cost.pending false)
                                                             :unknown (or total.unknown cost.unknown false)
                                                             :estimated (or total.estimated cost.estimated false)}))))))
                                  (fn group-item [owner part]
                                    (when (and owner (= owner.role :assistant) (not document-id))
                                      (when (not (. group-models owner.id))
                                        (local model (response-group db owner))
                                        (set model.label :Assistant)
                                        (local project (or misa.group_cost_projection misa.response_cost_projection))
                                        (when project (set model.cost (project db owner.id)))
                                        (tset group-models owner.id model))
                                      (table.insert items
                                                    {:id (.. "response:" owner.id ":" part)
                                                     :role (.. :transcript.group_ part) :chrome true
                                                     :model (. group-models owner.id)})))
                                  (each [_ source (ipairs blocks)]
                                    (local model {})
                                    (each [key value (pairs source)]
                                      (tset model key value))
                                    (when model.chunks
                                      (set model.text
                                           (table.concat model.chunks)))
                                    (local selected
                                           (and misa.selection_projection
                                                (misa.selection_projection db (selection-id model))))
                                    (local selecting
                                           (and selected
                                                (= selected.id
                                                   (selection-id model))))
                                    (when (and selecting model.text)
                                      (set model.text selected.text)
                                      ;; The presentation source is frozen. Live transport
                                      ;; chunks must not override it during syntax lookup.
                                      (set model.chunks nil))
                                    (when (and syntax-projections misa.syntax_projection)
                                      (local syntax
                                             (misa.syntax_projection syntax-projections model))
                                      (when syntax
                                        (set model.syntax syntax)))
                                    (local owner
                                           (or (. turn-owners model.response_id) (response db model.response_id)))
                                    (when (not= previous-owner owner)
                                      (group-item previous-owner :footer)
                                      (group-item owner :header)
                                      (set previous-owner owner))
                                    (local projector (. projectors model.kind))
                                    (local presentation (and projector (projector model state (and selecting selected))))
                                    (local role (and presentation presentation.role))
                                    (each [key value (pairs (or (and presentation presentation.model) {}))]
                                      (tset model key value))
                                    (when role
                                      (table.insert items {:id (selection-id model) : role : model})))
                                  (group-item previous-owner :footer)
                                  (local projection (misa.project_components db (if document-id :selection_geometry :transcript) items context-copy))
                                  (local result [])
                                  (each [index item (ipairs items)]
                                      (local model item.model)
                                      (local rendered (. projection.views index))
                                      (local lines (if (and misa.selection_decorate (not document-id) (not item.chrome))
                                                       (misa.selection_decorate db (selection-id model)
                                                                                  (or model.text model.result model.argument_text "")
                                                                                  (or rendered.lines []))
                                                       (or rendered.lines [])))
                                      (when (and (> (length result) 0) (> (length lines) 0) (not document-id))
                                        (table.insert result {:spans [] :transcript_id item.id :source_part :spacing}))
                                      (each [_ line (ipairs lines)]
                                        (table.insert result (misa.patch line {:transcript_id item.id :source_part (when item.chrome :chrome)})))
                                      (when (and (= item.role :transcript.group_footer) (= index (length items))
                                                 (> (length lines) 0) (not document-id))
                                        (table.insert result {:spans [] :transcript_id item.id :source_part :spacing}))
                                      (when (and misa.attachment_lines
                                                 model.attachments)
                                        (each [_ line (ipairs (misa.attachment_lines db
                                                                                     model.attachments
                                                                                     context-copy))]
                                          (tset result (+ (length result) 1)
                                                (misa.patch line {:transcript_id item.id :source_part :attachments})))))
                                  result)})
          (table.insert setup-fx {:type :register/service :name :transcript_viewport :value transcript-viewport})
          (table.insert setup-fx {:type :register/service :name :transcript_blocks :value transcript-blocks})
          (table.insert setup-fx
                        {:type :register/service :name :transcript_window
                         :value (fn [db context room] (. (transcript-viewport db context room) :lines))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/reset
                         :handler (fn [db]
                                    {:patch {:messages {:responses (misa.replace []) :blocks (misa.replace [])
                                                         :by_response (misa.replace {})
                                                         :scroll 0 :top misa.delete :anchor misa.delete :scroll_selection misa.delete}}})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/response-start
                         :handler (fn [db event cofx]
                                    (assert (and (= (type event.response_id)
                                                    :string)
                                                 (not= event.response_id ""))
                                            "response ID must be nonempty")
                                    (message-update (append-response db event.response_id
                                                                      (or event.role :assistant) cofx :streaming event) nil))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/block-start
                         :handler (fn [db event]
                                    (local owner
                                           (assert (response db
                                                             event.response_id)
                                                   "unknown transcript response"))
                                    (assert (= owner.status :streaming)
                                            "response is finalized")
                                    (assert (and (and (= (type event.block_id)
                                                         :string)
                                                      (not= event.block_id ""))
                                                 (not (find-block db owner
                                                                  event.block_id)))
                                            "invalid transcript block ID")
                                    (local kind
                                           (assert event.kind
                                                   "transcript block kind is missing"))
                                    (local block
                                           {:id event.block_id
                                            :interrupted false
                                            : kind
                                            :streaming true})
                                    (if (or (= kind :assistant)
                                            (= kind :thinking))
                                        (do
                                          (set block.chunks {})
                                          (set block.byte_count 0))
                                        (= kind :tool_call)
                                        (do
                                          (set block.name
                                               (printable-text (tostring (or event.name
                                                                             :tool))))
                                          (set block.call_id event.call_id)
                                          (set block.status :pending)
                                          (set block.argument_chunks {})
                                          (set block.argument_bytes 0))
                                        (error (.. "unsupported streaming transcript block: "
                                                   (tostring kind))))
                                    (message-update (append-block db owner block)
                                                    (updated-fx nil owner.id block.id)))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/block-delta
                         :handler (fn [db event]
                                    (local owner
                                           (assert (response db
                                                             event.response_id)
                                                   "unknown transcript response"))
                                    (local block
                                           (assert (find-block db owner
                                                               event.block_id)
                                                   "unknown transcript block"))
                                    (assert block.streaming
                                            "transcript block is finalized")
                                    (local handler (assert (. delta-handlers block.kind)
                                                           (.. "unsupported transcript delta: " block.kind)))
                                    (local patch (handler block event policy))
                                    (when patch (replace-transcript-block db block patch)))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/block-end
                         :handler (fn [db event]
                                    (local owner
                                           (assert (response db
                                                             event.response_id)
                                                   "unknown transcript response"))
                                    (local block (assert (find-block db owner event.block_id) "unknown transcript block"))
                                    (let [finished (finish-block block event policy)
                                          blocks (icollect [_ previous (ipairs db.messages.blocks)] (if (= previous block) finished previous))]
                                      {:patch {:messages {:blocks (misa.replace blocks)}}
                                       :fx (when (not= finished block)
                                             (updated-fx nil owner.id block.id))}))})
          (table.insert setup-fx
                        {:type :register/event :name :transcript/response-end
                         :handler (fn [db event cofx]
                                    (local previous (assert (response db event.response_id) "unknown transcript response"))
                                    (local (_ now) (clock cofx))
                                    (local (blocks finished) (finalized-response db previous now false policy))
                                    (local output (and (= (type event.usage) :table) event.usage.output_tokens))
                                    (local rate (and (= (type output) :number) (>= output 0) (> finished.elapsed_ms 0)))
                                    (var owner (misa.patch finished {:output_tokens (when rate output)
                                                                    :tokens_per_second (when rate (/ (* output 1000) finished.elapsed_ms))}))
                                    (local fx [])
                                    (when (= owner.role :assistant)
                                      (local text [])
                                      (for [index owner.block_start (- (+ owner.block_start owner.block_count) 1)]
                                        (local block (. blocks index))
                                        (when (= block.kind :assistant)
                                          (table.insert text (or block.text ""))))
                                      (when (> (length text) 0)
                                        (local committed {:text (table.concat text "") :started_wall_ms owner.started_wall_ms
                                                          :tokens_per_second owner.tokens_per_second})
                                        (each [_ effect (ipairs (noninteractive-commit db :transcript.assistant committed cofx markdown))]
                                          (table.insert fx effect))))
                                    (response-update db owner blocks fx))})
          (table.insert setup-fx
                        {:type :register/event :name :transcript/response-interrupted
                         :handler (fn [db event cofx]
                                    (local previous (response db event.response_id))
                                    (when previous
                                      (local (_ now) (clock cofx))
                                      (local (blocks owner) (finalized-response db previous now true policy))
                                      (response-update db owner blocks [])))})

          (fn standalone [db kind text event cofx extra]
            (local sequence (+ db.messages.next_id 1))
            (local id (.. :transcript- sequence))
            (local (next owner) (append-response (misa.patch db {:messages {:next_id sequence}})
                                                id (if (= kind :user) :user :system) cofx :complete))
            (append-block next owner
                          (misa.patch {:id (.. id :/1) :is_error (and event (= event.is_error true))
                                       : kind :level (and event event.level) :streaming false
                                       :text (printable-text (tostring (or text "")))}
                                      (or extra {}))))

          (each [_ kind (ipairs [:user :harness])]
            (table.insert setup-fx
                          {:type :register/event :name (.. :transcript/ kind)
                           :handler (fn [db event cofx]
                                      (local (next model) (standalone db kind event.text event cofx
                                                                     {:attachments (when (= kind :user) event.attachments)}))
                                      (message-update next
                                                      (updated-fx (noninteractive-commit next (.. :transcript. kind) model cofx markdown)
                                                                  model.response_id model.id)))}))
          (table.insert setup-fx
                        {:type :register/event :name :transcript/tool-start
                         :handler (fn [db event cofx]
                                    (local section (find-tool-section db event.id))
                                    (when (and section (= section.result nil) (not section.execution_started_monotonic_ms))
                                      (local (wall mono) (clock cofx))
                                      (replace-transcript-block db section
                                                                {:execution_started_wall_ms wall
                                                                 :execution_started_monotonic_ms mono :status :running})))})
          (table.insert setup-fx
                        {:type :register/event :name :transcript/tool-result
                         :handler (fn [db event cofx]
                                    (local section (find-tool-section db event.id))
                                    (local (_ now) (clock cofx))
                                    (local status (if (= event.cancelled true) :cancelled event.is_error :error :success))
                                    (if section
                                        (let [result (replace-transcript-block db section
                                                                              {:result (misa.replace (printable-text (tostring (or event.text ""))))
                                                                               :text (when (= section.kind :tool_result) (printable-text (tostring (or event.text ""))))
                                                                               :is_error (= event.is_error true) : status :streaming false
                                                                               :execution_completed_monotonic_ms (when section.execution_started_monotonic_ms (or section.execution_completed_monotonic_ms now))
                                                                               :elapsed_ms (when section.execution_started_monotonic_ms
                                                                                             (or section.elapsed_ms (math.max 0 (- now section.execution_started_monotonic_ms))))})]
                                          (tset result.patch.messages :scroll 0)
                                          result)
                                        (let [(next block) (standalone db :tool_result event.text event cofx {: status :call_id event.id})]
                                          (message-update next (updated-fx nil block.response_id block.id)))))})
          (table.insert setup-fx
                        {:type :register/event :name :transcript/tool-summary
                         :handler (fn [db event]
                                    (local section (find-tool-section db event.id))
                                    (when (and section (= (type event.text) :string))
                                      (replace-transcript-block db section {:summary event.text})))})
          (table.insert setup-fx
                        {:type :register/event :name :transcript/tool-call
                         :handler (fn [db event cofx]
                                    (local name (printable-text (tostring (or event.name :tool))))
                                    (local (next block) (standalone db :tool_call "" event cofx
                                                                {:call_id event.id : name
                                                                 :status :pending
                                                                 :arguments (misa.replace (copy-structural (or event.arguments event.arguments_json {}) policy 0))}))
                                    (message-update next (updated-fx nil block.response_id block.id)))})
          ;; Compatibility completion input for custom agents. It is normalized once
          ;; into the same response/block lifecycle rather than maintained as a shadow.
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/assistant
                         :handler (fn [db event cofx]
                                    (local id
                                           (or event.request_id
                                               (.. :legacy-
                                                   (tostring (+ db.messages.next_id
                                                                1)))))
                                    (local base (if event.request_id db (misa.patch db {:messages {:next_id (+ db.messages.next_id 1)}})))
                                    (local (created owner) (append-response base id :assistant cofx :complete))
                                    (var next created)
                                    (local output {})
                                    (each [index source (ipairs (or event.content
                                                                    {}))]
                                      (local kind
                                             (or (and (= source.type :text)
                                                      :assistant)
                                                 source.type))
                                      (local block
                                             {:arguments (and source.arguments
                                                              (copy-structural source.arguments
                                                                               policy
                                                                               0))
                                              :call_id source.id
                                              :id (.. id "/" index)
                                              : kind
                                              :name source.name
                                              :streaming false
                                              :text (and source.text
                                                         (printable-text source.text))})
                                      (when (= kind :tool_call)
                                        (set block.status :pending))
                                      (set next (append-block next owner block))
                                      (when (= kind :assistant)
                                        (tset output (+ (length output) 1)
                                              block.text)))
                                    (local fx {})
                                    (when (> (length output) 0)
                                      (each [_ effect (ipairs (noninteractive-commit db
                                                                                     :transcript.assistant
                                                                                     {:text (table.concat output
                                                                                                          "")
                                                                                      :started_wall_ms owner.started_wall_ms}
                                                                                     cofx
                                                                                     markdown))]
                                        (tset fx (+ (length fx) 1) effect)))
                                    (message-update next (if (> (. (response next id) :block_count) 0)
                                                            (updated-fx fx id) fx)))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/interrupted
                         :handler (fn [db event cofx]
                                    (var next db)
                                    (when (not (response next event.request_id))
                                      (local (created owner) (append-response next event.request_id :assistant cofx :streaming))
                                      (set next created)
                                      (each [index source (ipairs (or event.content []))]
                                        (set next (append-block next owner {:id (.. event.request_id "/" index)
                                                                           :kind (if (= source.type :text) :assistant source.type)
                                                                           :streaming false :text source.text}))))
                                    (local owner (response next event.request_id))
                                    (local responses (icollect [_ entry (ipairs next.messages.responses)]
                                                       (if (= entry.id owner.id) (misa.patch entry {:status :interrupted}) entry)))
                                    (local blocks (icollect [index block (ipairs next.messages.blocks)]
                                                    (if (and (>= index owner.block_start) (< index (+ owner.block_start owner.block_count)))
                                                        (misa.patch block {:interrupted true}) block)))
                                    (message-update (misa.patch next {:messages {:responses (misa.replace responses)
                                                                                :blocks (misa.replace blocks)}})
                                                    (when (> owner.block_count 0) (updated-fx nil owner.id))))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :runtime/dispatch-limit
                         :handler (fn [_ event]
                                    {:fx [{:type :dispatch
                                           :event {:type :transcript/harness
                                                   :level :error
                                                   :text event.text}}]})})
          {:fx setup-fx})}
