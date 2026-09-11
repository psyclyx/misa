;; Compaction policy contracts. Summarization runs as an independent, tool-free
;; background request; only a successful summary for an unchanged conversation
;; replaces canonical history, and the visible transcript is never rewritten.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local compaction (require :misa.compaction))

(local specs (require :misa.standard.compaction))
(local stock-costs (require :misa.standard.costs))
(local app ((require :tests.application) {:argv [] :config {}}))
(app.define (. (require :tests.stock) :misa.json))
(app.install)

(fn handler-for [type]
  (accumulate [found nil _ spec (pairs (. specs :events)) &until found]
    (when (= spec.event type) spec.handler)))

(fn transition [db event]
  (local before (misa.json.encode db))
  (local result ((handler-for event.type) db event {:config {}}))
  (assert (= before (misa.json.encode db))
          "compaction controller mutated prior state")
  (values (misa.patch db (or (and result result.patch) {}))
          (or (and result result.fx) [])))

(fn effect-of [fx kind]
  (accumulate [found nil _ effect (ipairs fx) &until found]
    (when (and (= (type effect) :table) (= (type effect.event) :table)
               (= (. effect.event :type) kind))
      effect.event)))

(fn provider-request [fx]
  (accumulate [found nil _ effect (ipairs fx) &until found]
    (when (= effect.type :provider.test) effect)))

(fn contains? [text fragment]
  (and (= (type text) :string) (not= nil (text:find fragment 1 true))))

(fn notice-of [fx]
  (effect-of fx :transcript/harness))

(fn notice-text [fx]
  (. (or (notice-of fx) {}) :text))

;; Cost accounting consumes compaction usage through the stock catalog.
(assert (. (. stock-costs :events) :costs/compaction/usage)
        "compaction usage is not wired into cost accounting")

;; Settings validation and documented defaults.
(local defaults (compaction.settings {}))
(assert (= defaults.enabled true))
(assert (= defaults.threshold 0.8))
(assert (= defaults.reserve_tokens 16000))
(assert (= defaults.min_messages 6))
(assert (= defaults.max_input_bytes 400000))
(assert (= defaults.max_summary_bytes 64000))
(local configured (compaction.settings {:compaction {:enabled false
                                                     :prompt "Summarize tersely."
                                                     :threshold 0.5}}))
(assert (= configured.enabled false))
(assert (= configured.prompt "Summarize tersely."))
(assert (= configured.min_messages defaults.min_messages))
(each [_ invalid (ipairs [{:threshold 0}
                          {:threshold 1.5}
                          {:enabled :yes}
                          {:reserve_tokens -1}
                          {:prompt 5}])]
  (local (ok _) (pcall compaction.settings {:compaction invalid}))
  (assert (not ok) "invalid compaction settings were accepted"))

;; Canonical history renders role-tagged plain text, including tool calls and
;; results, so the model sees paths, commands, and outcomes.
(local messages [{:role :user :content [{:type :text :text "fix the parser"}]}
                 {:role :assistant
                  :content [{:type :thinking :text "reading"}
                            {:type :text :text "Looking at it."}
                            {:type :tool_call
                             :name :shell
                             :arguments {:command "ls src"}}]}
                 {:role :tool :content [{:type :text :text "parser.zig"}]}])
(local history (compaction.history-text messages 10000))
(assert (contains? history "user: fix the parser"))
(assert (contains? history "assistant: reading\nLooking at it."))
(assert (contains? history "tool call shell {command=ls src}"))
(assert (contains? history "tool: parser.zig"))

;; A conversation larger than the input budget keeps its beginning and its most
;; recent work, marks the omission, and never splits a UTF-8 sequence.
(local wide (string.rep "é" 4000))
(local bounded (compaction.history-text [{:role :user
                                          :content [{:type :text :text wide}]}
                                         {:role :assistant
                                          :content [{:type :text :text "tail marker"}]}]
                                        4000))
(assert (<= (length bounded) 4000))
(assert (contains? bounded "[earlier conversation omitted]"))
(assert (contains? bounded "tail marker"))
(assert (= (select 2 (bounded:gsub "\xc3" ""))
           (select 2 (bounded:gsub "\xa9" ""))))

;; The estimate prefers a provider-reported request total and otherwise uses a
;; byte heuristic over the rendered history.
(assert (= (compaction.estimate-tokens {:agent {:messages messages}}) 27))
(assert (= (compaction.estimate-tokens {:agent {:messages messages}
                                        :usage {:last_request {:input_tokens 900
                                                               :output_tokens 100}}})
           1000))
(assert (= (compaction.estimate-tokens {}) 0))

;; The budget follows the selected model's context window and the message floor.
(local catalog {:models {:selected :test/model
                         :entries [{:id :test/model
                                    :provider :test
                                    :model :model
                                    :context_window 200000}
                                   {:id :test/other
                                    :provider :test
                                    :model :other
                                    :context_window 200000}]}})
(assert (= (compaction.due? (compaction.settings {}) catalog) false))
(local many [(. messages 1)
             (. messages 2)
             (. messages 3)
             (. messages 1)
             (. messages 2)
             (. messages 3)])
(local almost {:models catalog.models
               :agent {:messages many :status :ready}
               :usage {:last_request {:input_tokens 150000 :output_tokens 1000}}})
(local without-model
       (misa.patch almost {:models {:selected misa.delete}}))
(assert (= (compaction.due? (compaction.settings {}) almost) true))
(assert (= (compaction.due? (compaction.settings {:compaction {:enabled false}})
                            almost)
           false))

;; Automatic compaction starts only from a ready agent above the budget, never
;; from compaction's own completion, and never for a one-shot argv run.
(local (_ check-fx) (transition almost {:type :agent/status :status :ready}))
(assert (= (length check-fx) 1))
(assert (= (. check-fx 1 :event :type) :compaction/start))
(assert (= (. check-fx 1 :event :automatic) true))
(assert (= (transition almost {:type :agent/status
                               :status :ready
                               :compaction true})
           almost))
(local one-shot (misa.patch almost {:agent {:exit_after_response true}}))
(assert (= (transition one-shot {:type :agent/status :status :ready}) one-shot))
(local busy-agent (misa.patch almost {:agent {:status :working}}))
(local (not-ready not-ready-fx)
       (transition busy-agent {:type :agent/status :status :ready}))
(assert (= not-ready busy-agent))
(assert (= (length not-ready-fx) 0))
(local attempted (misa.patch almost
                                {:compaction {:signature (compaction.signature almost)}}))
(local (retried retried-fx)
       (transition attempted {:type :agent/status :status :ready}))
(assert (= retried attempted))
(assert (= (length retried-fx) 0))
;; Nothing is queued while no model is selected, so an unconfigured
;; session does not repeat a refusal on every turn.
(local (unassigned unassigned-fx)
       (transition without-model {:type :agent/status :status :ready}))
(assert (= unassigned without-model))
(assert (= (length unassigned-fx) 0))

;; Starting a request requires a ready agent, a selected model, and
;; something to compact; each refusal names its reason and changes no state.
(local (busy busy-fx) (transition busy-agent {:type :compaction/start}))
(assert (= busy busy-agent))
(assert (= (. (notice-of busy-fx) :level) :warning))
(assert (contains? (notice-text busy-fx) "agent is busy"))
(local empty {:agent {:messages [] :status :ready}})
(assert (= (transition empty {:type :compaction/start}) empty))
(local (missing missing-fx) (transition without-model {:type :compaction/start}))
(assert (= missing without-model))
(assert (= (. (notice-of missing-fx) :level) :warning))
(assert (contains? (notice-text missing-fx) "available model"))
(assert (contains? (notice-text missing-fx) "/model"))

;; A successful start issues one tool-free provider request holding the rendered
;; conversation, the handoff prompt, and a progress status.
(local (started start-fx) (transition almost {:type :compaction/start}))
(assert (= (. start-fx 1 :event :type) :agent/status))
(assert (= (. start-fx 1 :event :status) :compacting))
(assert (= (notice-of start-fx) nil))
(assert (= (length start-fx) 2))
(local request (provider-request start-fx))
(assert (= request.model :model))
(assert (= (length request.tools) 0))
(assert (= (length request.messages) 1))
(assert (= (. request.messages 1 :role) :user))
(assert (contains? request.system_prompt "handoff note"))
(assert (contains? (. request.messages 1 :content 1 :text)
                   "user: fix the parser"))
(assert (= started.compaction.active.id request.id))
(assert (= started.compaction.active.model :test/model))
(assert (= started.compaction.active.dialog false))
(assert (= started.compaction.active.automatic false))
(assert (= (misa.json.encode started.compaction.signature)
           (misa.json.encode (compaction.signature almost))))
(assert (= started.compaction.sequence 1))

;; An explicit /compact marks the request as interactive and opens a cancellable
;; progress dialog; typed argument text reaches the summarization prompt.
(local (dialog dialog-fx) (transition almost {:type :compaction/start
                                              :dialog true
                                              :instructions "focus on the parser API"}))
(local open (effect-of dialog-fx :dialog/open))
(assert (= open.id :compaction))
(assert (= open.kind :progress))
(assert (= open.cancellable true))
(assert (= open.completion :compaction/action))
(assert (= open.correlation :compaction))
(assert (= open.title "Compacting conversation"))
(assert (contains? open.message "6 messages"))
(assert (= dialog.compaction.active.dialog true))
(local dialog-request (provider-request dialog-fx))
(assert (contains? dialog-request.system_prompt
                   "Additional instructions from the user:\nfocus on the parser API"))
(assert (= (transition dialog {:type :compaction/start}) dialog))

;; Streamed summary text accumulates up to the configured bound while unrelated
;; streams and non-text deltas are ignored, and usage is captured.
(local request-id started.compaction.active.id)
(local streamed (transition started {:type :agent/stream-delta
                                     :id request-id
                                     :delta {:type :text :text "Goal: parse."}}))
(assert (= streamed.compaction.active.summary "Goal: parse."))
(assert (= (transition streamed {:type :agent/stream-delta
                                 :id :other
                                 :delta {:type :text :text "ignored"}})
           streamed))
(assert (= (transition streamed {:type :agent/stream-delta
                                 :id request-id
                                 :delta {:type :thinking :text "ignored"}})
           streamed))
(local clipped (transition started {:type :agent/stream-delta
                                    :id request-id
                                    :delta {:type :text
                                            :text (string.rep "a" 70000)}}))
(assert (= (length clipped.compaction.active.summary) 64000))
(local used (transition streamed {:type :agent/stream-usage
                                  :id request-id
                                  :usage {:input_tokens 900 :output_tokens 40}}))
(assert (= used.compaction.active.usage.output_tokens 40))

;; A completed summary replaces canonical history with a user handoff and the
;; assistant continuation, records its usage, and restores the ready status.
(local (applied applied-fx) (transition used {:type :agent/stream-end
                                              :id request-id}))
(assert (= (length applied.agent.messages) 2))
(assert (= (. applied.agent.messages 1 :role) :user))
(assert (= (. applied.agent.messages 2 :role) :assistant))
(assert (contains? (. applied.agent.messages 1 :content 1 :text) "Goal: parse."))
(assert (contains? (. applied.agent.messages 1 :content 1 :text) "handoff note"))
(assert (= applied.compaction.active nil))
(assert (= applied.compaction.sequence 1))
(local reported (effect-of applied-fx :compaction/usage))
(assert (= reported.usage.output_tokens 40))
(assert (= reported.model :test/model))
(assert (= reported.response_id request-id))
(assert (= (. (effect-of applied-fx :agent/status) :status) :ready))
(assert (= (. (effect-of applied-fx :agent/status) :compaction) true))
(assert (= (. (effect-of applied-fx :dialog/close) :id) :compaction))
(assert (= (. (notice-of applied-fx) :level) :info))
(assert (contains? (notice-text applied-fx) "Compacted 6 messages"))
(assert (= (transition applied {:type :agent/stream-end :id request-id}) applied))

;; A conversation that changed while summarizing is never rewritten, and the
;; discard is reported rather than silently applied.
(local changed (misa.patch used {:agent {:messages (misa.replace messages)}}))
(local (discarded discarded-fx) (transition changed {:type :agent/stream-end
                                                     :id request-id}))
(assert (= (length discarded.agent.messages) 3))
(assert (= (. discarded.agent.messages 1 :content 1 :text) "fix the parser"))
(assert (= discarded.compaction.active nil))
(assert (= (. (notice-of discarded-fx) :level) :warning))
(assert (contains? (notice-text discarded-fx) "conversation changed"))
(assert (= (. (effect-of discarded-fx :agent/status) :status) :ready))

;; An empty summary, one no shorter than its input, a failed request, and a
;; selected model that disappeared are all refused without touching history.
(local (blank blank-fx) (transition started {:type :agent/stream-end
                                             :id request-id}))
(assert (= blank.agent almost.agent))
(assert (= blank.compaction.active nil))
(assert (contains? (notice-text blank-fx) "no usable summary"))
(local long-summary (misa.patch started
                                {:compaction {:active {:summary (string.rep "x" 500)}}}))
(local (unhelpful unhelpful-fx) (transition long-summary {:type :agent/stream-end
                                                          :id request-id}))
(assert (= unhelpful.agent almost.agent))
(assert (= unhelpful.compaction.active nil))
(assert (contains? (notice-text unhelpful-fx) "no shorter summary"))
(local (failed failed-fx) (transition streamed {:type :agent/stream-error
                                                :id request-id}))
(assert (= failed.agent almost.agent))
(assert (= (. (notice-of failed-fx) :level) :error))
(assert (contains? (notice-text failed-fx) "Compaction failed"))
(local (unavailable _)
       (transition (misa.patch streamed {:models {:selected misa.delete}})
                   {:type :agent/stream-end :id request-id}))
(assert (= unavailable.agent almost.agent))
(assert (= unavailable.compaction.active nil))

;; Cancellation stops the request, restores the ready status, and leaves history
;; untouched; reconciliation cancels only when the selected model changed.
(local (cancelled cancel-fx) (transition started {:type :compaction/cancel}))
(assert (= cancelled.compaction.active nil))
(assert (= (. cancel-fx 1 :type) :operation/cancel))
(assert (= (. cancel-fx 1 :id) request-id))
(assert (= (. (effect-of cancel-fx :agent/status) :status) :ready))
(assert (= (. (notice-of cancel-fx) :level) :info))
(assert (= (transition cancelled {:type :compaction/cancel}) cancelled))
(local (interrupted _) (transition started {:type :agent/cancel-active}))
(assert (= interrupted.compaction.active nil))
(local (quiet quiet-fx) (transition started {:type :compaction/cancel
                                             :reason :quiet}))
(assert (= (notice-of quiet-fx) nil))
(assert (= (. specs :events :compaction/reconcile :event) :compaction/reconcile))
(local (reconciled reconcile-fx) (transition started
                                            {:type :compaction/reconcile}))
(assert (= reconciled started))
(assert (= (length reconcile-fx) 0))
(assert (= (notice-of reconcile-fx) nil))
(assert (= (transition almost {:type :compaction/reconcile}) almost))
(local (stale _)
       (transition (misa.patch started {:models {:selected :test/other}})
                   {:type :compaction/reconcile}))
(assert (= stale.compaction.active nil))

;; Reset clears bookkeeping while keeping the request sequence monotonic, so a
;; later request reuses the incrementing identity.
(local (reset reset-fx) (transition started {:type :transcript/reset}))
(assert (= reset.compaction.active nil))
(assert (= reset.compaction.sequence 1))
(assert (= (. reset-fx 1 :type) :operation/cancel))
(local (restarted _) (transition reset {:type :compaction/start}))
(assert (= (tostring restarted.compaction.active.id) "compaction-2"))
(assert (= restarted.compaction.sequence 2))

;; Draft submission is held only while a compaction request is running, and the
;; command and action declarations follow the stock shapes.
(assert (= (misa.json.encode (. specs :services :editor.lifecycle.compaction))
           (misa.json.encode [:compaction/lifecycle])))
(assert (= (. specs :subscriptions :compaction/lifecycle :compute)
           compaction.lifecycle))
(assert (= (. (compaction.lifecycle [nil]) :block_draft) false))
(assert (= (. (compaction.lifecycle [{}]) :block_draft) true))
(assert (= (. specs :commands :/compact :event) :compaction/request))
(assert (= (. specs :actions :compaction.compact :event :type) :compaction/request))
(local requested ((handler-for :compaction/request) nil {:arguments "focus"}))
(assert (= (. requested :fx 1 :type) :dispatch))
(local requested-event (. requested :fx 1 :event))
(assert (= requested-event.type :compaction/start))
(assert (= requested-event.dialog true))
(assert (= requested-event.instructions "focus"))

(output "compaction policy contracts passed\n")
