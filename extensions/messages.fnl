;; Canonical transcript models and projection. Interactive output is composed

;; only from db.messages.transcript by the root managed view.

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

(fn append-response [db id role cofx status]
  (let [state (assert db.messages "message state is not initialized")]
    (assert (not (. state.by_response id))
            (.. "duplicate transcript response: " (tostring id)))
    (local (wall mono) (clock cofx))
    (local response {:block_count 0
                     :block_start (+ (length state.blocks) 1)
                     : id
                     : role
                     :started_monotonic_ms mono
                     :started_wall_ms wall
                     :status (or status :streaming)})
    (tset state.responses (+ (length state.responses) 1) response)
    (tset state.by_response id (length state.responses))
    (set state.scroll 0)
    response))

(fn response [db id]
  (let [state db.messages
        index (and state (. state.by_response id))]
    (or (and index (. state.responses index)) nil)))

(fn append-block [db response-model block]
  (let [state db.messages]
    (set block.response_id response-model.id)
    (set block.role response-model.role)
    (set block.started_wall_ms response-model.started_wall_ms)
    (tset state.blocks (+ (length state.blocks) 1) block)
    (set state.transcript state.blocks)
    (set response-model.block_count (+ response-model.block_count 1))
    (set state.scroll 0)
    block))

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
    (when (and (and (= block.kind :tool_call) (= block.call_id call-id))
               (= block.result nil))
      (lua "return block")))
  nil)

(fn tool-description [name]
  (let [tool (and misa.tool (misa.tool name))]
    (or (and tool (printable-text tool.description)) nil)))

;; Structural tool previews have a size budget; conversation text does not.
(fn append-delta [block value]
  (local text (printable-text (tostring (or value ""))))
  (when (not= text "")
    (table.insert block.chunks text)
    (set block.byte_count (+ (or block.byte_count 0) (length text)))))

(fn finish-block [block event policy]
  (when block.chunks (set block.text (table.concat block.chunks))
    (set block.chunks nil))
  (when (and event (not= event.arguments nil))
    (set block.arguments (copy-structural event.arguments policy 0)))
  (when (and event (not= event.name nil))
    (set block.name (printable-text (tostring event.name)))
    (when (= block.kind :tool_call)
      (set block.description (tool-description block.name))))
  (when (and event (not= event.call_id nil))
    (set block.call_id (printable-text (tostring event.call_id))))
  (if (not= block.arguments nil)
      (do
        (set block.argument_chunks nil)
        (set block.argument_text nil))
      block.argument_chunks
      (do
        (set block.argument_text (table.concat block.argument_chunks))
        (set block.argument_chunks nil)))
  (set block.argument_bytes nil)
  (set block.streaming false)
  nil)

(fn timestamp [ms]
  (let [seconds (% (math.floor (/ (or (tonumber ms) 0) 1000)) 86400)]
    (string.format "%02d:%02d:%02d" (math.floor (/ seconds 3600))
                   (% (math.floor (/ seconds 60)) 60) (% seconds 60))))

(fn selection-id [block]
  (let [owner (tostring (or block.response_id ""))]
    (.. (length owner) ":" owner (tostring block.id))))

{:setup (fn [context]
          (local setup-fx [])
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
                                   :value (fn [db]
                                            (or (and (. (or db.messages {})
                                                        :verbose)
                                                     :verbose)
                                                :summary))}}))
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (set db.messages
                                         {:blocks {}
                                          :by_response {}
                                          :next_id 0
                                          :responses {}
                                          :scroll 0
                                          :transcript {}
                                          :verbose (= config.verbose true)})
                                    (set db.messages.transcript
                                         db.messages.blocks)
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :messages/toggle-verbose
                         :handler (fn [db]
                                    (set (db.messages.verbose db.messages.scroll)
                                         (values (not db.messages.verbose) 0))
                                    {: db
                                     :fx [{:event {:type :ui/redraw}
                                           :type :dispatch}]})})
          (var viewport {:first 1 :room 0 :total 0})
          (table.insert setup-fx
                        {:type :register/event
                         :name :messages/scroll
                         :handler (fn [db event]
                                    (local bottom
                                           (math.max 1
                                                     (+ (- viewport.total
                                                           viewport.room)
                                                        1)))
                                    (local first
                                           (math.max 1
                                                     (math.min bottom
                                                               (- viewport.first
                                                                  event.delta))))
                                    (set db.messages.top
                                         (or (and (< first bottom) first) nil))
                                    (set db.messages.scroll
                                         (math.max 0 (- bottom first)))
                                    (set viewport.first first)
                                    {: db :fx [{:type :terminal/read}]})})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (when (and (and (= tx.event.type
                                                              :terminal/input)
                                                           (not tx.db.picker))
                                                      misa.keybinding_action)
                                             (local action
                                                    (misa.keybinding_action :global
                                                                            tx.event))
                                             (if (or (= tx.event.kind :wheel_up)
                                                     (= tx.event.kind
                                                        :wheel_down))
                                                 (set tx.event
                                                      {:delta (or (and (= tx.event.kind
                                                                          :wheel_up)
                                                                       3)
                                                                  (- 3))
                                                       :type :messages/scroll})
                                                 (= action :toggle_verbose)
                                                 (set tx.event
                                                      {:type :messages/toggle-verbose})
                                                 (= action :transcript_up)
                                                 (set tx.event
                                                      {:delta (math.max 1
                                                                        (math.floor (/ tx.cofx.terminal.lines
                                                                                       2)))
                                                       :type :messages/scroll})
                                                 (= action :transcript_down)
                                                 (set tx.event
                                                      {:delta (- (math.max 1
                                                                           (math.floor (/ tx.cofx.terminal.lines
                                                                                          2))))
                                                       :type :messages/scroll})))
                                           tx)
                                 :id :messages/global-keys}})
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
                           :value (fn [db]
                                    (local result {})
                                    (each [_ block (ipairs (or (. (or db.messages
                                                                      {})
                                                                  :blocks)
                                                               {}))]
                                      (var text
                                           (or (or (or block.text
                                                       (and block.chunks
                                                            (table.concat block.chunks)))
                                                   block.result)
                                               block.argument_text))
                                      (when (and (not text) block.arguments)
                                        (set text (describe block.arguments 0)))
                                      (when (and text (not= text ""))
                                        (tset result (+ (length result) 1)
                                              (misa.selection_document (selection-id block)
                                                                       (.. block.kind
                                                                           " · "
                                                                           (timestamp block.started_wall_ms)
                                                                           " — "
                                                                           (misa.layout.clip (or (text:match "[^\r
]+")
                                                                                                 "")
                                                                                             40))
                                                                       text))))
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
                                  (local (result state)
                                         (values {}
                                                 (assert db.messages
                                                         "message state is not initialized")))
                                  (each [_ source (ipairs state.blocks)]
                                    (local model {})
                                    (each [key value (pairs source)]
                                      (tset model key value))
                                    (when model.chunks
                                      (set model.text
                                           (table.concat model.chunks)))
                                    (local selected
                                           (and misa.selection_projection
                                                (misa.selection_projection db)))
                                    (local selecting
                                           (and selected
                                                (= selected.id
                                                   (selection-id model))))
                                    (when (and selecting model.text)
                                      (set model.text selected.text))
                                    (when misa.syntax_projection
                                      (local syntax
                                             (misa.syntax_projection db model))
                                      ;; Immutable projection inputs stay shared across the component
                                      ;; model snapshot; resolving this value performs no work.
                                      (when syntax
                                        (set model.syntax (fn [] syntax))))
                                    (set model.timestamp
                                         (timestamp model.started_wall_ms))
                                    (local owner
                                           (response db model.response_id))
                                    (when (and owner
                                               (= owner.metadata_block_id
                                                  model.id))
                                      (set model.tokens_per_second
                                           owner.tokens_per_second))
                                    (when (and (and owner
                                                    (= owner.metadata_block_id
                                                       model.id))
                                               misa.response_cost_projection)
                                      (local cost
                                             (misa.response_cost_projection db
                                                                            model.response_id))
                                      (set model.cost (and cost cost.text)))
                                    (var role nil)
                                    (if (= model.kind :user)
                                        (set role :transcript.user)
                                        (= model.kind :assistant)
                                        (set role :transcript.assistant)
                                        (= model.kind :thinking)
                                        (do
                                          (set role
                                               (or (and (or (or state.verbose
                                                                model.streaming)
                                                            selecting)
                                                        :transcript.thinking)
                                                   :transcript.thinking_collapsed))
                                          (set model.summary
                                               (or (and model.streaming
                                                        :streaming)
                                                   :summary)))
                                        (= model.kind :tool_call)
                                        (do
                                          (set role :transcript.tool_call)
                                          (set model.detail
                                               (or (and (or state.verbose
                                                            selecting)
                                                        (or (and model.arguments
                                                                 (describe model.arguments
                                                                           0))
                                                            (printable-text (or model.argument_text
                                                                                (table.concat (or model.argument_chunks
                                                                                                  {}))))))
                                                   :summary))
                                          (set model.result_detail
                                               (or (and state.verbose
                                                        model.result)
                                                   (or (and (not= model.result
                                                                  nil)
                                                            :summary)
                                                       nil)))
                                          (when selecting
                                            (if (not= model.result nil)
                                                (do
                                                  (set model.result_detail
                                                       selected.text)
                                                  (set model.selection_source
                                                       :result))
                                                (do
                                                  (set model.detail
                                                       selected.text)
                                                  (set model.selection_source
                                                       :args)))))
                                        (= model.kind :tool_result)
                                        (do
                                          (set role :transcript.tool_result)
                                          (set model.collapsed
                                               (not (or state.verbose selecting))))
                                        (= model.kind :harness)
                                        (set role :transcript.harness))
                                    (if (= model.kind :user)
                                        (set model.rail :rail.user)
                                        (= model.kind :assistant)
                                        (set model.rail :rail.assistant)
                                        (= model.kind :thinking)
                                        (set model.rail :rail.thinking)
                                        (or (= model.kind :tool_call)
                                            (= model.kind :tool_result))
                                        (set model.rail
                                             (or (and model.is_error
                                                      :rail.error)
                                                 :rail.tool))
                                        (= model.kind :harness)
                                        (set model.rail
                                             (or (and (= model.level :error)
                                                      :rail.error)
                                                 :rail.harness)))
                                    (when role
                                      (local rendered
                                             (misa.render_component db role
                                                                    model
                                                                    context-copy))
                                      (when misa.selection_decorate
                                        (set rendered.lines
                                             (misa.selection_decorate db
                                                                      (selection-id model)
                                                                      (or (or (or model.text
                                                                                  model.result)
                                                                              model.argument_text)
                                                                          "")
                                                                      (or rendered.lines
                                                                          {}))))
                                      (each [_ line (ipairs (or rendered.lines
                                                                {}))]
                                        (tset result (+ (length result) 1) line))
                                      (when (and misa.attachment_lines
                                                 model.attachments)
                                        (each [_ line (ipairs (misa.attachment_lines db
                                                                                     model.attachments
                                                                                     context-copy))]
                                          (tset result (+ (length result) 1)
                                                line)))))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :transcript_window
                         :value (fn [db render-context available-lines]
                                  (local room
                                         (math.max 0
                                                   (math.floor (or available-lines
                                                                   0))))
                                  (if (= room 0) {}
                                      (do
                                        (local lines
                                               (misa.transcript_projection db
                                                                           render-context))
                                        (local selected
                                               (and misa.selection_projection
                                                    (misa.selection_projection db)))
                                        (local selected-key
                                               (and selected
                                                    (.. selected.id ":"
                                                        selected.first ":"
                                                        selected.last)))
                                        (var first
                                             (math.min (or (or db.messages.top
                                                               (and (and selected-key
                                                                         (= viewport.selection
                                                                            selected-key))
                                                                    viewport.first))
                                                           (math.max 1
                                                                     (+ (- (length lines)
                                                                           room)
                                                                        1)))
                                                       (math.max 1
                                                                 (+ (- (length lines)
                                                                       room)
                                                                    1))))
                                        (local result {})
                                        (when (and selected-key
                                                   (not= viewport.selection
                                                         selected-key))
                                          (each [index line (ipairs lines)]
                                            (when line.selected
                                              (set first
                                                   (math.max 1
                                                             (math.min first
                                                                       index)))
                                              (when (>= index (+ first room))
                                                (set first (+ (- index room) 1)))
                                              (lua :break))))
                                        (set viewport
                                             {: first
                                              : room
                                              :selection selected-key
                                              :total (length lines)})
                                        (for [index first (math.min (length lines)
                                                                    (- (+ first
                                                                          room)
                                                                       1))]
                                          (tset result (+ (length result) 1)
                                                (. lines index)))
                                        result)))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/reset
                         :handler (fn [db]
                                    (set (db.messages.responses db.messages.blocks
                                                                db.messages.by_response)
                                         (values {} {} {}))
                                    (set db.messages.transcript
                                         db.messages.blocks)
                                    (set db.messages.scroll 0)
                                    (set db.messages.top nil)
                                    (set viewport {:first 1 :room 0 :total 0})
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/response-start
                         :handler (fn [db event cofx]
                                    (assert (and (= (type event.response_id)
                                                    :string)
                                                 (not= event.response_id ""))
                                            "response ID must be nonempty")
                                    (append-response db event.response_id
                                                     (or event.role :assistant)
                                                     cofx :streaming)
                                    {: db})})
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
                                          (set block.description
                                               (tool-description block.name))
                                          (set block.call_id event.call_id)
                                          (set block.status :pending)
                                          (set block.argument_chunks {})
                                          (set block.argument_bytes 0))
                                        (error (.. "unsupported streaming transcript block: "
                                                   (tostring kind))))
                                    (append-block db owner block)
                                    {: db})})
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
                                    (if (or (= block.kind :assistant)
                                            (= block.kind :thinking))
                                        (append-delta block event.text)
                                        (do
                                          (when (not= event.name nil)
                                            (set block.name
                                                 (printable-text (tostring event.name))))
                                          (when (not= event.call_id nil)
                                            (set block.call_id
                                                 (printable-text (tostring event.call_id))))
                                          (when (not= event.arguments_json nil)
                                            (set block.argument_chunks {})
                                            (set block.argument_bytes 0)
                                            (set block.arguments_truncated nil)
                                            (set event.arguments_json_delta
                                                 event.arguments_json))
                                          (when (and (not= event.arguments_json_delta
                                                           nil)
                                                     (not block.arguments_truncated))
                                            (local value
                                                   (printable-text (tostring event.arguments_json_delta)))
                                            (local remaining
                                                   (- policy.max_string
                                                      (or block.argument_bytes
                                                          0)))
                                            (if (<= remaining 0)
                                                (do
                                                  (tset block.argument_chunks
                                                        (+ (length block.argument_chunks)
                                                           1)
                                                        "… [truncated]")
                                                  (set block.arguments_truncated
                                                       true))
                                                (> (length value) remaining)
                                                (do
                                                  (tset block.argument_chunks
                                                        (+ (length block.argument_chunks)
                                                           1)
                                                        (truncate-text value
                                                                       remaining))
                                                  (set block.argument_bytes
                                                       policy.max_string)
                                                  (set block.arguments_truncated
                                                       true))
                                                (do
                                                  (tset block.argument_chunks
                                                        (+ (length block.argument_chunks)
                                                           1)
                                                        value)
                                                  (set block.argument_bytes
                                                       (+ (or block.argument_bytes
                                                              0)
                                                          (length value))))))
                                          (when (not= event.arguments nil)
                                            (set block.arguments
                                                 (copy-structural event.arguments
                                                                  policy 0)))))
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/block-end
                         :handler (fn [db event]
                                    (local owner
                                           (assert (response db
                                                             event.response_id)
                                                   "unknown transcript response"))
                                    (finish-block (assert (find-block db owner
                                                                      event.block_id)
                                                          "unknown transcript block")
                                                  event policy)
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/response-end
                         :handler (fn [db event cofx]
                                    (local owner
                                           (assert (response db
                                                             event.response_id)
                                                   "unknown transcript response"))
                                    (local (_ now) (clock cofx))
                                    (for [index owner.block_start (- (+ owner.block_start
                                                                        owner.block_count)
                                                                     1)]
                                      (local block (. db.messages.blocks index))
                                      (when block.streaming
                                        (finish-block block nil policy)))
                                    (set owner.status :complete)
                                    (set owner.completed_monotonic_ms now)
                                    (set owner.elapsed_ms
                                         (math.max 0
                                                   (- now
                                                      owner.started_monotonic_ms)))
                                    (local output
                                           (or (and (= (type event.usage)
                                                       :table)
                                                    event.usage.output_tokens)
                                               nil))
                                    (when (and (and (= (type output) :number)
                                                    (>= output 0))
                                               (> owner.elapsed_ms 0))
                                      (set owner.output_tokens output)
                                      (set owner.tokens_per_second
                                           (/ (* output 1000) owner.elapsed_ms)))
                                    (local fx {})
                                    (when (= owner.role :assistant)
                                      (var (text last-text) {})
                                      (for [i owner.block_start (- (+ owner.block_start
                                                                      owner.block_count)
                                                                   1)]
                                        (local block (. db.messages.blocks i))
                                        (when (= block.kind :assistant)
                                          (tset text (+ (length text) 1)
                                                (or block.text ""))
                                          (set last-text block)))
                                      (when last-text
                                        (set owner.metadata_block_id
                                             last-text.id))
                                      (when (> (length text) 0)
                                        (local committed
                                               {:text (table.concat text "")
                                                :timestamp (timestamp owner.started_wall_ms)
                                                :tokens_per_second owner.tokens_per_second})
                                        (each [_ effect (ipairs (noninteractive-commit db
                                                                                       :transcript.assistant
                                                                                       committed
                                                                                       cofx
                                                                                       markdown))]
                                          (tset fx (+ (length fx) 1) effect))))
                                    {: db : fx})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/response-interrupted
                         :handler (fn [db event cofx]
                                    (local owner
                                           (response db event.response_id))
                                    (if (not owner) {: db}
                                        (do
                                          (local (_ now) (clock cofx))
                                          (set owner.status :interrupted)
                                          (set owner.completed_monotonic_ms now)
                                          (set owner.elapsed_ms
                                               (math.max 0
                                                         (- now
                                                            owner.started_monotonic_ms)))
                                          (for [i owner.block_start (- (+ owner.block_start
                                                                          owner.block_count)
                                                                       1)]
                                            (local block
                                                   (. db.messages.blocks i))
                                            (finish-block block nil policy)
                                            (set block.interrupted true))
                                          {: db})))})

          (fn standalone [db kind text event cofx]
            (set db.messages.next_id (+ db.messages.next_id 1))
            (local id (.. :transcript- db.messages.next_id))
            (local owner
                   (append-response db id
                                    (or (and (= kind :user) :user) :system) cofx
                                    :complete))
            (local model
                   (append-block db owner
                                 {:id (.. id :/1)
                                  :is_error (and event (= event.is_error true))
                                  : kind
                                  :level (and event event.level)
                                  :streaming false
                                  :text (if (or (= kind :tool_call)
                                                (= kind :tool_result))
                                            (copy-structural (tostring (or text
                                                                           ""))
                                                             policy 0)
                                            (printable-text (tostring (or text
                                                                          ""))))}))
            model)

          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/user
                         :handler (fn [db event cofx]
                                    (local model
                                           (standalone db :user event.text
                                                       event cofx))
                                    (set model.attachments event.attachments)
                                    {: db
                                     :fx (noninteractive-commit db
                                                                :transcript.user
                                                                model cofx
                                                                markdown)})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/tool-result
                         :handler (fn [db event cofx]
                                    (local section
                                           (find-tool-section db event.id))
                                    (if section
                                        (do
                                          (set section.result
                                               (copy-structural (tostring (or event.text
                                                                              ""))
                                                                policy 0))
                                          (set section.is_error
                                               (= event.is_error true))
                                          (set section.status
                                               (or (and (= event.cancelled true)
                                                        :cancelled)
                                                   (or (and section.is_error
                                                            :error)
                                                       :success)))
                                          (set section.streaming false)
                                          (set db.messages.scroll 0))
                                        (do
                                          (local fallback
                                                 (standalone db :tool_result
                                                             event.text event
                                                             cofx))
                                          (set fallback.status
                                               (or (and (= event.cancelled true)
                                                        :cancelled)
                                                   (or (and event.is_error
                                                            :error)
                                                       :success)))))
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/harness
                         :handler (fn [db event cofx]
                                    (local model
                                           (standalone db :harness event.text
                                                       event cofx))
                                    {: db
                                     :fx (noninteractive-commit db
                                                                :transcript.harness
                                                                model cofx
                                                                markdown)})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/tool-call
                         :handler (fn [db event cofx]
                                    (local model
                                           (standalone db :tool_call "" event
                                                       cofx))
                                    (set model.call_id event.id)
                                    (set model.name
                                         (printable-text (tostring (or event.name
                                                                       :tool))))
                                    (set model.description
                                         (tool-description model.name))
                                    (set model.status :pending)
                                    (set model.arguments
                                         (copy-structural (or (or event.arguments
                                                                  event.arguments_json)
                                                              {})
                                                          policy 0))
                                    {: db})})
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
                                    (local owner
                                           (append-response db id :assistant
                                                            cofx :complete))
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
                                        (set block.status :pending)
                                        (set block.description
                                             (tool-description block.name)))
                                      (append-block db owner block)
                                      (when (= kind :assistant)
                                        (tset output (+ (length output) 1)
                                              block.text)))
                                    (local fx {})
                                    (when (> (length output) 0)
                                      (each [_ effect (ipairs (noninteractive-commit db
                                                                                     :transcript.assistant
                                                                                     {:text (table.concat output
                                                                                                          "")
                                                                                      :timestamp (timestamp owner.started_wall_ms)}
                                                                                     cofx
                                                                                     markdown))]
                                        (tset fx (+ (length fx) 1) effect)))
                                    {: db : fx})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/interrupted
                         :handler (fn [db event cofx]
                                    (local result {: db})
                                    (when (not (response db event.request_id))
                                      (local owner
                                             (append-response db
                                                              event.request_id
                                                              :assistant cofx
                                                              :streaming))
                                      (each [index source (ipairs (or event.content
                                                                      {}))]
                                        (append-block db owner
                                                      {:id (.. event.request_id
                                                               "/" index)
                                                       :kind (or (and (= source.type
                                                                         :text)
                                                                      :assistant)
                                                                 source.type)
                                                       :streaming false
                                                       :text source.text})))
                                    (local owner (response db event.request_id))
                                    (set owner.status :interrupted)
                                    (for [i owner.block_start (- (+ owner.block_start
                                                                    owner.block_count)
                                                                 1)]
                                      (tset (. db.messages.blocks i)
                                            :interrupted true))
                                    result)})
          (table.insert setup-fx
                        {:type :register/event
                         :name :runtime/dispatch-limit
                         :handler (fn [_ event]
                                    {:fx [{:type :dispatch
                                           :event {:type :transcript/harness
                                                   :level :error
                                                   :text event.text}}]})})
          {:fx setup-fx})}
