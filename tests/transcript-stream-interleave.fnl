;; A standalone transcript message may arrive while a response is still streaming:
;; a harness notice, a compaction report, a queued user turn. Each response must keep
;; owning a contiguous window of blocks, so a block appended after such a message still
;; resolves and its stream deltas keep arriving.
;;
;; The ordering is what makes this reachable. complete-response publishes the ready
;; status and then the completion, so an automatic compaction starts from the ready
;; status while the queued prompt drains from the completion that follows it. The
;; compaction request then finishes against a busy agent and reports the discard
;; into the transcript while the queued turn is still streaming.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local stock (require :tests.stock))

(app.define (. stock :misa.json))
(app.define (. stock :misa.transcript))
(app.define (. stock :misa.agent))
(app.define (. stock :misa.agent.stream))
(app.define (. stock :misa.compaction))
(app.define (. stock :misa.editor.queue))
(app.define (. stock :misa.models))

(var observed nil)
(app.define {:events {:probe/seed {:event :probe/seed
                                   :handler (fn [_ event] {:patch event.value})}
                      :probe/read {:event :probe/read
                                   :handler (fn [db _]
                                              (set observed db)
                                              nil)}}})
(app.install)

;; Dispatch one transaction and stage its dispatches at the queue tail, as a session
;; does. Provider requests announce their stream synchronously, so the queued
;; stream-start is staged here too.
(var pending [])
(var requested [])
(fn step [event]
  (let [effects (misa._dispatch event
                                {:columns 80 :lines 24 :interactive false}
                                {:wall_ms 0 :monotonic_ms 0})]
    (misa._commit)
    (each [_ effect (ipairs effects)]
      (if (= effect.type :dispatch)
          (table.insert pending effect.event)
          (do
            (table.insert requested effect)
            (when (and (= (type effect.type) :string)
                       (= (type effect.id) :string)
                       (effect.type:match "^provider%."))
              (table.insert pending {:type :agent/stream-start :id effect.id}))))))
  nil)

(fn settle []
  (while (> (length pending) 0)
    (step (table.remove pending 1)))
  nil)

(fn read-state []
  (step {:type :probe/read})
  (settle)
  observed)

(fn text-of [block]
  (or block.text block.result block.argument_text
      (and block.chunks (table.concat block.chunks))
      (and block.argument_chunks (table.concat block.argument_chunks))))

(fn notice? [block]
  (and (= block.kind :harness) (not= nil (text-of block))))

;; Every response owns exactly its own blocks, and by_response agrees with the array.
(fn check-windows [state]
  (each [index owner (ipairs state.messages.responses)]
    (assert (= (. state.messages.by_response owner.id) index)
            "response index drifted from by_response")
    (for [position owner.block_start (- (+ owner.block_start owner.block_count) 1)]
      (assert (= (. state.messages.blocks position :response_id) owner.id)
              (.. "block " (tostring position) " is outside the window of "
                  (tostring owner.id)))))
  (each [_ block (ipairs state.messages.blocks)]
    (assert (= (type block.response_id) :string)
            "a block lost its owning response"))
  true)

(fn has-block? [state id]
  (accumulate [found false _ block (ipairs state.messages.blocks) &until found]
    (= block.id id)))

(fn contains? [text fragment]
  (and (= (type text) :string) (not= nil (text:find fragment 1 true))))

(step {:type :app/start})
(settle)

(local conversation [{:role :user :content [{:type :text :text "one"}]}
                     {:role :assistant :content [{:type :text :text "two"}]}
                     {:role :user :content [{:type :text :text "three"}]}
                     {:role :assistant :content [{:type :text :text "four"}]}
                     {:role :user :content [{:type :text :text "five"}]}
                     {:role :assistant :content [{:type :text :text "six"}]}
                     {:role :user :content [{:type :text :text "seven"}]}])

(step {:type :probe/seed
       :value {:models {:selected :test/model
                        :entries [{:id :test/model
                                   :context_window 200000
                                   :provider :test
                                   :model :model}
                                  {:id :test/small
                                   :context_window 200000
                                   :provider :test
                                   :model :small}]
                        }
               :agent {:messages conversation :request_seq 1 :status :ready}
               :usage {:last_request {:input_tokens 190000 :output_tokens 1000}}
               :queue {:pending "queued prompt" :attachments {} :sending false}}})
(settle)

;; The previous turn is already visible in the transcript, as any live session has it.
(step {:type :transcript/response-start :response_id :agent-1 :role :assistant})
(step {:type :transcript/block-start
       :response_id :agent-1
       :block_id :agent-1/1
       :kind :assistant})
(step {:type :transcript/block-delta
       :response_id :agent-1
       :block_id :agent-1/1
       :text "seven"})
(step {:type :transcript/block-end :response_id :agent-1 :block_id :agent-1/1})
(step {:type :transcript/response-end :response_id :agent-1})
(settle)
(assert (has-block? (read-state) :agent-1/1) "the previous turn is not in the transcript")

;; A turn completes over budget with a prompt waiting: the ready status starts
;; compaction, and the completion that follows releases the queued turn.
(table.insert pending {:type :agent/status :status :ready})
(table.insert pending {:type :agent/completed :exit false})
(settle)

(local running (read-state))
(assert (not= nil running.compaction.active) "automatic compaction did not start")
(assert (= running.compaction.active.id :compaction-1) "compaction booked no request")
(assert (= running.agent.status :working) "the queued prompt was not released")
(assert (= running.agent.active_request_id :agent-2) "the queued turn did not start")
(assert (= running.queue.pending "") "the queued prompt was not taken")
(local compaction-request
       (accumulate [found nil _ effect (ipairs requested) &until found]
         (when (and (= effect.type :provider.test) (= effect.id :compaction-1))
           effect)))
(assert (not= nil compaction-request) "the compaction request was not issued")
(check-windows running)

;; The released turn streams its first block.
(step {:type :agent/stream-delta :id :agent-2 :delta {:type :text :text "first"}})
(settle)
(local streaming (read-state))
(assert (has-block? streaming :agent-2/1) "the first block was not created")

;; The compaction request finishes against a busy agent, so it is discarded and
;; reported into the transcript while the released turn is still streaming.
(step {:type :agent/stream-delta
       :id :compaction-1
       :delta {:type :text :text "Goal: the work so far."}})
(step {:type :agent/stream-end :id :compaction-1})
(settle)
(local reported (read-state))
(assert (not (contains? (. reported.agent.messages 1 :content 1 :text) "handoff"))
        "a discarded compaction replaced canonical history")
(assert (= (length reported.agent.messages) 8)
        "the released turn did not add its own prompt")
(assert (= nil reported.compaction.active)
        "the discarded compaction stayed active")
(local reported-notices (icollect [_ block (ipairs reported.messages.blocks)]
                          (when (notice? block) block)))
(local discarded (accumulate [found nil _ block (ipairs reported-notices) &until found]
                   (when (contains? (text-of block) "another turn") block)))
(assert (not= nil discarded) "the discarded compaction was not reported")
(check-windows reported)

;; The released turn adds a second block. Its deltas must still resolve, and both
;; blocks must stay inside the response that owns them.
(step {:type :agent/stream-delta
       :id :agent-2
       :delta {:type :tool_call :index 0 :id :call :name :tool :arguments_json "{}"}})
(settle)
(local continued (read-state))
(assert (has-block? continued :agent-2/2) "the second block was not created")
(local owner (. continued.messages.responses (. continued.messages.by_response
                                                    :agent-2)))
(assert (= owner.block_count 2) "the response did not take ownership of its block")
(check-windows continued)
(let [window (icollect [index block (ipairs continued.messages.blocks)]
               (when (and (>= index owner.block_start)
                          (< index (+ owner.block_start owner.block_count)))
                 block.id))]
  (assert (= (table.concat window ",") "agent-2/1,agent-2/2")
          "the response window is not its own contiguous blocks"))

;; A notice that arrives while a response streams is ordered after that response's
;; blocks, so it cannot displace a block outside its window.
(local last-block (. continued.messages.blocks (length continued.messages.blocks)))
(assert (= last-block.kind :harness)
        "the notice did not follow the streamed blocks it arrived during")

(output "transcript stream interleave contracts passed\n")
