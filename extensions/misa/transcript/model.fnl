(fn lower [value] (: (tostring value) :lower))

(fn printable-text [value]
  (var (out i byte-length) (values {} 1 (length value)))

  (fn continuation [byte]
    (and byte (>= byte 128) (<= byte 191)))

  (while (<= i byte-length)
    (let [byte (value:byte i)]
      (if (= byte 27) (let [next-byte (value:byte (+ i 1))]
                        (if (= next-byte 91)
                            (do
                              (set i (+ i 2))
                              (do
                                (var finished? false)
                                (while (and (not finished?) (<= i byte-length))
                                  (let [current (value:byte i)]
                                    (set i (+ i 1))
                                    (when (and (>= current 64) (<= current 126))
                                      (set finished? true))))))
                            (= next-byte 93)
                            (do
                              (set i (+ i 2))
                              (do
                                (var finished? false)
                                (while (and (not finished?) (<= i byte-length))
                                  (let [current (value:byte i)]
                                    (when (= current 7) (set i (+ i 1))
                                      (set finished? true))
                                    (when (not finished?)
                                      (when (and (= current 27)
                                                 (= (value:byte (+ i 1)) 92))
                                        (set i (+ i 2))
                                        (set finished? true))
                                      (when (not finished?) (set i (+ i 1))))))))
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
            (set i (+ i 1))) (or (< byte 32) (= byte 127))
          (set i (+ i 1)) (let [width (or (and (< byte 128) 1)
                                         (and (>= byte 194) (<= byte 223) 2)
                                         (and (>= byte 224) (<= byte 239) 3)
                                         (and (>= byte 240) (<= byte 244) 4) 0)
                               (b2 b3 b4) (values (value:byte (+ i 1))
                                                  (value:byte (+ i 2))
                                                  (value:byte (+ i 3)))
                               valid (or (= width 1)
                                         (and (= width 2) (continuation b2))
                                         (and (= width 3) (continuation b2)
                                              (continuation b3)
                                              (not (and (= byte 224) (< b2 160)))
                                              (not (and (= byte 237) (> b2 159))))
                                         (and (= width 4) (continuation b2)
                                              (continuation b3)
                                              (continuation b4)
                                              (not (and (= byte 240) (< b2 144)))
                                              (not (and (= byte 244) (> b2 143)))))]
                           (if valid
                               (do
                                 (when (not (and (= byte 194) (>= b2 128)
                                                 (<= b2 159)))
                                   (tset out (+ (length out) 1)
                                         (value:sub i (- (+ i width) 1))))
                                 (set i (+ i width)))
                               (do
                                 (tset out (+ (length out) 1) "�")
                                 (set i (+ i 1))))))))
  (table.concat out))

(fn truncate-text [value limit]
  (if (<= (length value) limit) value
      (do
        (var boundary limit)
        (while (and (> boundary 0) (value:byte (+ boundary 1))
                    (>= (value:byte (+ boundary 1)) 128)
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
                      (do
                        (var finished? false)
                        (each [child-key child (pairs value) &until finished?]
                          (set count (+ count 1))
                          (when (> count policy.max_items)
                            (tset result "…" "[truncated items]")
                            (set finished? true))
                          (when (not finished?)
                            (tset result child-key
                                  (copy-structural child policy (+ depth 1)
                                                   child-key)))))
                      result)))))))

(fn describe [value depth]
  "Describe a bounded structural value as printable text."
  (let [kind (type value)]
    (if (= kind :string) (printable-text value)
        (if (not= kind :table) (printable-text (tostring value))
            (if (> depth 3)
                "{…}"
                (let [source-values {}]
                  (each [key child (pairs value)]
                    (tset source-values (+ (length source-values) 1)
                          (.. (printable-text (tostring key)) "="
                              (describe child (+ depth 1)))))
                  (table.sort source-values)
                  (.. "{" (table.concat source-values ", ") "}")))))))

(fn clock [cofx]
  (let [value (assert cofx.clock "native clock coeffect is missing")]
    (values value.wall_ms value.monotonic_ms)))

(fn appended [items value]
  (let [next (icollect [_ item (ipairs items)] item)]
    (table.insert next value)
    next))

(fn updated-fx [fx response-id block-id]
  (appended (or fx []) {:type :dispatch
                        :event {:type :transcript/updated
                                :response_id response-id
                                :block_id block-id}}))

(fn message-update [db fx]
  {:patch {:messages (misa.replace db.messages)} : fx})

(fn append-response [db id role cofx status timing]
  (let [state (assert db.messages "message state is not initialized")]
    (assert (not (. state.by_response id))
            (.. "duplicate transcript response: " (tostring id)))
    (let [(wall mono) (clock cofx)
          owner {:block_count 0
                 :block_start (+ (length state.blocks) 1)
                 : id
                 : role
                 :started_monotonic_ms (or (and timing
                                                timing.started_monotonic_ms)
                                           mono)
                 :started_wall_ms (or (and timing timing.started_wall_ms) wall)
                 :status (or status :streaming)}]
      (values (misa.patch db
                          {:messages {:responses (misa.replace (appended state.responses
                                                                         owner))
                                      :by_response {id (+ (length state.responses)
                                                          1)}
                                      :scroll 0}}) owner))))

(fn response [db id]
  "Find a response by identity."
  (let [state db.messages
        index (and state state.by_response (. state.by_response id))]
    (and index (. state.responses index))))

(fn blocks [db response-id block-id]
  "Return the semantic transcript blocks for presentation."
  ;; Nil owner is an explicit full-transcript lookup for imports/fixture sync.
  ;; Scoped lookups allocate only the array; its records remain canonical.
  (let [blocks (or (and db.messages db.messages.blocks) [])]
    (if (= response-id nil) blocks (let [owner (response db response-id)
                                         result []]
                                     (when owner
                                       (for [index owner.block_start (- (+ owner.block_start
                                                                           owner.block_count)
                                                                        1)]
                                         (let [block (. blocks index)]
                                           (when (and block
                                                      (or (= block-id nil)
                                                          (= block.id block-id)))
                                             (table.insert result block)))))
                                     result))))

(fn response-group [db owner]
  "Describe response timing and associated tool activity."
  (var completed owner.completed_monotonic_ms)
  (var pending false)
  (each [_ block (ipairs (blocks db owner.id))]
    (when (= block.kind :tool_call)
      (when (or (= block.status :pending) (= block.status :running))
        (set pending true))
      (when block.execution_completed_monotonic_ms
        (set completed
             (math.max (or completed block.execution_completed_monotonic_ms)
                       block.execution_completed_monotonic_ms)))))
  (let [status (if (and (= owner.status :complete) pending) :running
                   owner.status)]
    {:started_wall_ms owner.started_wall_ms
     : status
     :elapsed_ms (when (and completed (not= status :streaming)
                            (not= status :running))
                   (math.max 0 (- completed owner.started_monotonic_ms)))
     :tokens_per_second owner.tokens_per_second}))

(fn append-block [db response-model block]
  (let [owner (assert (response db response-model.id)
                      "unknown transcript response")
        next-block (misa.patch block
                               {:response_id owner.id
                                :role owner.role
                                :started_wall_ms owner.started_wall_ms})
        responses (icollect [_ previous (ipairs db.messages.responses)]
                    (if (= previous.id owner.id)
                        (misa.patch previous
                                    {:block_count (+ owner.block_count 1)})
                        previous))]
    (values (misa.patch db {:messages {:blocks (misa.replace (appended db.messages.blocks
                                                                       next-block))
                                       :responses (misa.replace responses)
                                       :scroll 0}}) next-block)))

(fn find-block [db response-model id]
  (let [blocks db.messages.blocks
        after (+ response-model.block_start response-model.block_count)]
    (var index response-model.block_start)
    (var found nil)
    (while (and (< index after) (not found))
      (when (and (. blocks index) (= (. blocks index :id) id))
        (set found (. blocks index)))
      (set index (+ index 1)))
    found))

(fn find-tool-section [db call-id]
  (var found nil)
  (for [index (length db.messages.blocks) 1 -1 &until found]
    (let [block (. db.messages.blocks index)]
      (when (and (= block.call_id call-id)
                 (or (= block.kind :tool_call) (= block.kind :tool_result)))
        (set found block))))
  found)

(fn append-chunk [chunks text]
  (let [next (icollect [_ chunk (ipairs (or chunks []))]
               chunk)]
    (table.insert next text)
    next))

(fn text-delta [block event]
  "Append printable text while retaining earlier streaming chunks."
  (let [text (printable-text (tostring (or event.text "")))]
    (when (not= text "")
      {:chunks (misa.replace (append-chunk block.chunks text))
       :byte_count (+ (or block.byte_count 0) (length text))})))

(fn tool-delta [previous event policy]
  "Accumulate bounded tool arguments from a stream delta."
  (let [replacement (not= event.arguments_json nil)
        block (if replacement
                  (misa.patch previous
                              {:argument_chunks (misa.replace [])
                               :argument_bytes 0
                               :arguments_truncated misa.delete})
                  previous)
        raw (if replacement event.arguments_json event.arguments_json_delta)
        value (when (not= raw nil) (printable-text (tostring raw)))
        remaining (- policy.max_string (or block.argument_bytes 0))
        active (and value (not block.arguments_truncated))
        truncated (and active
                       (or (<= remaining 0) (> (length value) remaining)))
        chunk (when active
                (if (<= remaining 0) "… [truncated]"
                    truncated (truncate-text value remaining)
                    value))]
    {:name (when (not= event.name nil) (printable-text (tostring event.name)))
     :call_id (when (not= event.call_id nil)
                (printable-text (tostring event.call_id)))
     :argument_chunks (when (or active replacement)
                        (misa.replace (if active
                                          (append-chunk block.argument_chunks
                                                        chunk)
                                          block.argument_chunks)))
     :argument_bytes (when (or active replacement)
                       (if truncated policy.max_string
                           active (+ (or block.argument_bytes 0) (length value))
                           0))
     :arguments_truncated (if truncated true
                              replacement misa.delete
                              nil)
     :arguments (when (not= event.arguments nil)
                  (misa.replace (copy-structural event.arguments policy 0)))}))

(fn replace-transcript-block [db block patch]
  (let [replacement (misa.patch block patch)
        blocks (icollect [_ previous (ipairs db.messages.blocks)]
                 (if (= previous block) replacement previous))]
    {:patch {:messages {:blocks (misa.replace blocks)}}
     :fx (when (not= replacement block)
           (updated-fx nil block.response_id block.id))}))

(fn finish-block [block event policy]
  (let [value (or event {})
        name (when (not= value.name nil)
               (printable-text (tostring value.name)))
        has-arguments (or (not= value.arguments nil) (not= block.arguments nil))]
    (misa.patch block {:description misa.delete
                       :text (when block.chunks (table.concat block.chunks))
                       :chunks misa.delete
                       :arguments (when (not= value.arguments nil)
                                    (misa.replace (copy-structural value.arguments
                                                                   policy 0)))
                       :name name
                       :call_id (when (not= value.call_id nil)
                                  (printable-text (tostring value.call_id)))
                       :argument_text (if has-arguments misa.delete
                                          block.argument_chunks
                                          (table.concat block.argument_chunks)
                                          nil)
                       :argument_chunks misa.delete
                       :argument_bytes misa.delete
                       :streaming false})))

(fn finalized-response [db owner now interrupted policy]
  (let [blocks (icollect [index block (ipairs db.messages.blocks)]
                 (if (and (>= index owner.block_start)
                          (< index (+ owner.block_start owner.block_count)))
                     (let [finished (if (or interrupted block.streaming)
                                        (finish-block block nil policy)
                                        block)]
                       (if interrupted
                           (misa.patch finished {:interrupted true})
                           finished))
                     block))]
    (values blocks
            (misa.patch owner
                        {:status (if interrupted :interrupted :complete)
                         :completed_monotonic_ms now
                         :elapsed_ms (math.max 0
                                               (- now
                                                  owner.started_monotonic_ms))}))))

(fn response-update [db owner blocks fx]
  (let [responses (icollect [_ previous (ipairs db.messages.responses)]
                    (if (= previous.id owner.id) owner previous))]
    {:patch {:messages {:responses (misa.replace responses)
                        :blocks (misa.replace blocks)}}
     :fx (if (> owner.block_count 0) (updated-fx fx owner.id) fx)}))

(fn selection-id [block]
  "Return the stable selection identity of a transcript block."
  (let [owner (tostring (or block.response_id ""))]
    (.. (length owner) ":" owner (tostring block.id))))

(fn standalone [db kind text event cofx extra]
  (let [sequence (+ db.messages.next_id 1)
        id (.. :transcript- sequence)
        (next owner) (append-response (misa.patch db
                                                  {:messages {:next_id sequence}})
                                      id (if (= kind :user) :user :system) cofx
                                      :complete)]
    (append-block next owner
                  (misa.patch {:id (.. id :/1)
                               :is_error (and event (= event.is_error true))
                               : kind
                               :level (and event event.level)
                               :streaming false
                               :text (printable-text (tostring (or text "")))}
                              (or extra {})))))

(fn initialize [config db]
  "Create initial transcript state from configuration."
  {:patch {:messages (misa.replace {:blocks []
                                    :by_response {}
                                    :next_id 0
                                    :responses []
                                    :scroll 0
                                    :verbose (= config.verbose true)})}})

(fn toggle-detail [db]
  "Toggle transcript detail and return to the latest content."
  {:patch {:messages {:verbose (not db.messages.verbose) :scroll 0}}
   :fx [{:event {:type :ui/redraw} :type :dispatch}]})

(fn reset [db]
  "Clear transcript content and viewport state."
  {:patch {:messages {:responses (misa.replace [])
                      :blocks (misa.replace [])
                      :by_response (misa.replace {})
                      :scroll 0
                      :top misa.delete
                      :anchor misa.delete
                      :scroll_selection misa.delete}}})

(fn start-response [db event cofx]
  "Create an active response with native clock coordinates."
  (assert (and (= (type event.response_id) :string) (not= event.response_id ""))
          "response ID must be nonempty")
  (message-update (append-response db event.response_id
                                   (or event.role :assistant) cofx :streaming
                                   event) nil))

(fn start-block [db event]
  "Append a new block to an active response."
  (let [owner (assert (response db event.response_id)
                      "unknown transcript response")]
    (assert (= owner.status :streaming) "response is finalized")
    (assert (and (= (type event.block_id) :string) (not= event.block_id "")
                 (not (find-block db owner event.block_id)))
            "invalid transcript block ID")
    (let [kind (assert event.kind "transcript block kind is missing")
          block {:id event.block_id :interrupted false : kind :streaming true}]
      (if (or (= kind :assistant) (= kind :thinking))
          (do
            (set block.chunks {})
            (set block.byte_count 0))
          (= kind :tool_call)
          (do
            (set block.name (printable-text (tostring (or event.name :tool))))
            (set block.call_id event.call_id)
            (set block.status :pending)
            (set block.argument_chunks {})
            (set block.argument_bytes 0))
          (error (.. "unsupported streaming transcript block: " (tostring kind))))
      (message-update (append-block db owner block)
                      (updated-fx nil owner.id block.id)))))

(fn update-block [handlers policy db event]
  "Apply a streaming delta using the supplied kind handlers."
  (let [owner (assert (response db event.response_id)
                      "unknown transcript response")
        block (assert (find-block db owner event.block_id)
                      "unknown transcript block")]
    (assert block.streaming "transcript block is finalized")
    (let [handler (assert (. handlers block.kind)
                          (.. "unsupported transcript delta: " block.kind))
          patch (handler block event policy)]
      (when patch
        (replace-transcript-block db block patch)))))

(fn finish-streaming-block [policy db event]
  "Finalize a streaming block without changing its identity."
  (let [owner (assert (response db event.response_id)
                      "unknown transcript response")
        block (assert (find-block db owner event.block_id)
                      "unknown transcript block")
        finished (finish-block block event policy)
        blocks (icollect [_ previous (ipairs db.messages.blocks)]
                 (if (= previous block)
                     finished
                     previous))]
    {:patch {:messages {:blocks (misa.replace blocks)}}
     :fx (when (not= finished block)
           (updated-fx nil owner.id block.id))}))

(fn interrupt-response [policy db event cofx]
  "Finalize an existing response as interrupted."
  (let [previous (response db event.response_id)]
    (when previous
      (let [(_ now) (clock cofx)
            (blocks owner) (finalized-response db previous now true policy)]
        (response-update db owner blocks [])))))

(fn start-tool [db event cofx]
  "Record the first execution time for a pending tool."
  (let [section (find-tool-section db event.id)]
    (when (and section (= section.result nil)
               (not section.execution_started_monotonic_ms))
      (let [(wall mono) (clock cofx)]
        (replace-transcript-block db section
                                  {:execution_started_wall_ms wall
                                   :execution_started_monotonic_ms mono
                                   :status :running})))))

(fn finish-tool [db event cofx]
  "Record tool output and completion timing."
  (let [section (find-tool-section db event.id)
        (_ now) (clock cofx)
        status (if (= event.cancelled true) :cancelled
                   event.is_error :error
                   :success)]
    (if section
        (let [result (replace-transcript-block db section
                                               {:result (misa.replace (printable-text (tostring (or event.text
                                                                                                    ""))))
                                                :text (when (= section.kind
                                                               :tool_result)
                                                        (printable-text (tostring (or event.text
                                                                                      ""))))
                                                :is_error (= event.is_error
                                                             true)
                                                : status
                                                :streaming false
                                                :execution_completed_monotonic_ms (when section.execution_started_monotonic_ms
                                                                                    (or section.execution_completed_monotonic_ms
                                                                                        now))
                                                :elapsed_ms (when section.execution_started_monotonic_ms
                                                              (or section.elapsed_ms
                                                                  (math.max 0
                                                                            (- now
                                                                               section.execution_started_monotonic_ms))))})]
          (tset result.patch.messages :scroll 0)
          result)
        (let [(next block) (standalone db :tool_result event.text event cofx
                                       {: status :call_id event.id})]
          (message-update next (updated-fx nil block.response_id block.id))))))

(fn summarize-tool [db event]
  "Attach a summary to an existing tool call."
  (let [section (find-tool-section db event.id)]
    (when (and section (= (type event.text) :string))
      (replace-transcript-block db section {:summary event.text}))))

(fn append-tool [policy db event cofx]
  "Append a standalone tool call with bounded argument data."
  (let [name (printable-text (tostring (or event.name :tool)))
        (next block) (standalone db :tool_call "" event cofx
                                 {:call_id event.id
                                  : name
                                  :status :pending
                                  :arguments (misa.replace (copy-structural (or event.arguments
                                                                                event.arguments_json
                                                                                {})
                                                                            policy
                                                                            0))})]
    (message-update next (updated-fx nil block.response_id block.id))))

(fn append-interrupted [db event cofx]
  "Retain partial response content and mark it interrupted."
  (var next db)
  (when (not (response next event.request_id))
    (let [(created owner) (append-response next event.request_id :assistant
                                           cofx :streaming)]
      (set next created)
      (each [index source (ipairs (or event.content []))]
        (set next (append-block next owner
                                {:id (.. event.request_id "/" index)
                                 :kind (if (= source.type :text)
                                           :assistant
                                           source.type)
                                 :streaming false
                                 :text source.text})))))
  (let [owner (response next event.request_id)
        responses (icollect [_ entry (ipairs next.messages.responses)]
                    (if (= entry.id owner.id)
                        (misa.patch entry {:status :interrupted})
                        entry))
        blocks (icollect [index block (ipairs next.messages.blocks)]
                 (if (and (>= index owner.block_start)
                          (< index (+ owner.block_start owner.block_count)))
                     (misa.patch block {:interrupted true})
                     block))]
    (message-update (misa.patch next
                                {:messages {:responses (misa.replace responses)
                                            :blocks (misa.replace blocks)}})
                    (when (> owner.block_count 0)
                      (updated-fx nil owner.id)))))

(fn finish-response [commit markdown policy db event cofx]
  "Finalize response timing and plan noninteractive output."
  (let [previous (assert (response db event.response_id)
                         "unknown transcript response")
        (_ now) (clock cofx)
        (blocks finished) (finalized-response db previous now false policy)
        output (and (= (type event.usage) :table) event.usage.output_tokens)
        rate (and (= (type output) :number) (>= output 0)
                  (> finished.elapsed_ms 0))]
    (var owner
         (misa.patch finished
                     {:output_tokens (when rate
                                       output)
                      :tokens_per_second (when rate
                                           (/ (* output 1000)
                                              finished.elapsed_ms))}))
    (let [fx []]
      (when (= owner.role :assistant)
        (let [text []]
          (for [index owner.block_start (- (+ owner.block_start
                                              owner.block_count)
                                           1)]
            (let [block (. blocks index)]
              (when (= block.kind :assistant)
                (table.insert text (or block.text "")))))
          (when (> (length text) 0)
            (let [committed {:text (table.concat text "")
                             :started_wall_ms owner.started_wall_ms
                             :tokens_per_second owner.tokens_per_second}]
              (each [_ effect (ipairs (commit db :transcript.assistant
                                              committed cofx markdown))]
                (table.insert fx effect))))))
      (response-update db owner blocks fx))))

(fn append-assistant [commit markdown policy db event cofx]
  "Normalize a completed assistant response into transcript blocks."
  (let [id (or event.request_id
               (.. :legacy- (tostring (+ db.messages.next_id 1))))
        base (if event.request_id db
                 (misa.patch db
                             {:messages {:next_id (+ db.messages.next_id 1)}}))
        (created owner) (append-response base id :assistant cofx :complete)]
    (var next created)
    (let [output {}]
      (each [index source (ipairs (or event.content {}))]
        (let [kind (or (and (= source.type :text) :assistant) source.type)
              block {:arguments (and source.arguments
                                     (copy-structural source.arguments policy 0))
                     :call_id source.id
                     :id (.. id "/" index)
                     : kind
                     :name source.name
                     :streaming false
                     :text (and source.text (printable-text source.text))}]
          (when (= kind :tool_call)
            (set block.status :pending))
          (set next (append-block next owner block))
          (when (= kind :assistant)
            (tset output (+ (length output) 1) block.text))))
      (let [fx {}]
        (when (> (length output) 0)
          (each [_ effect (ipairs (commit db :transcript.assistant
                                          {:text (table.concat output "")
                                           :started_wall_ms owner.started_wall_ms}
                                          cofx markdown))]
            (tset fx (+ (length fx) 1) effect)))
        (message-update next (if (> (. (response next id) :block_count) 0)
                                 (updated-fx fx id)
                                 fx))))))

(fn append-message [commit markdown kind db event cofx]
  "Append a standalone message and plan its presentation effects."
  (let [(next model) (standalone db kind event.text event cofx
                                 {:attachments (when (= kind :user)
                                                 event.attachments)})]
    (message-update next
                    (updated-fx (commit next (.. :transcript. kind) model cofx
                                        markdown)
                                model.response_id model.id))))

{: initialize
 : describe
 : toggle-detail
 : response
 : response-group
 : selection-id
 : append-message
 : text-delta
 : tool-delta
 : append-assistant
 : update-block
 : finish-streaming-block
 : start-block
 : blocks
 : append-interrupted
 : reset
 : finish-response
 : interrupt-response
 : start-response
 : append-tool
 : finish-tool
 : start-tool
 : summarize-tool}
