;; Installed-cascade contracts for conversation compaction. The pure policy lives in
;; tests/compaction.fnl; this file installs the stock wiring and drives real dispatch,
;; because those fixtures supply state the application never provides (models.entries,
;; a resolvable summarizer) and so cannot show that an unconfigured session is inert.
;; Nothing here is a performance claim.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local stock (require :tests.stock))
(local compaction (require :misa.compaction))
(app.define (. stock :misa.json))
(app.define (. stock :misa.models))
(app.define (. stock :misa.compaction))

(var observed nil)
(app.define {:events {:probe/seed {:event :probe/seed
                                   :handler (fn [_ event] {:patch event.value})}
                      :probe/read {:event :probe/read
                                   :handler (fn [db _]
                                              ;; Runs inside the dispatch, so the
                                              ;; subscription scope is live here.
                                              (set observed
                                                   {:db db
                                                    :lifecycle (misa.sub db
                                                                         [:compaction/lifecycle])})
                                              nil)}}})
(app.install)

(fn step [event]
  (local effects (misa._dispatch event
                                 {:columns 80 :lines 24 :interactive false}
                                 {:wall_ms 0 :monotonic_ms 0}))
  (misa._commit)
  effects)

;; Drive every queued dispatch effect, collecting the effects and events observed at
;; every depth. Effects run after their model transaction commits, as in a session.
(fn settle [event]
  (let [result {:effects [] :events []}]
    (fn walk [fx depth]
      (when (< depth 12)
        (each [_ effect (ipairs fx)]
          (table.insert result.effects effect)
          (when (= effect.type :dispatch)
            (table.insert result.events effect.event)
            (walk (step effect.event) (+ depth 1))))))
    (walk (step event) 0)
    result))

(fn event-of [result kind]
  (accumulate [found nil _ event (ipairs result.events) &until found]
    (when (= event.type kind) event)))

(fn effect-of [result kind]
  (accumulate [found nil _ effect (ipairs result.effects) &until found]
    (when (= effect.type kind) effect)))

(fn seen? [result kind]
  (not= (event-of result kind) nil))

(fn contains? [text fragment]
  (and (= (type text) :string) (not= nil (text:find fragment 1 true))))

(fn notice-text [result]
  (let [notice (event-of result :transcript/harness)]
    (. (or notice {}) :text)))

(fn read-state []
  (step {:type :probe/read})
  observed)

(fn messages-of []
  (let [agent (read-state)]
    (or (and agent.db.agent agent.db.agent.messages) [])))

(local messages [{:role :user :content [{:type :text :text "one"}]}
                 {:role :assistant :content [{:type :text :text "two"}]}
                 {:role :user :content [{:type :text :text "three"}]}
                 {:role :assistant :content [{:type :text :text "four"}]}
                 {:role :user :content [{:type :text :text "five"}]}
                 {:role :assistant :content [{:type :text :text "six"}]}
                 {:role :user :content [{:type :text :text "seven"}]}])

(fn seed [{:roles roles :window window}]
  {:models {:selected :test/model
            :entries [(if window
                          {:id :test/model
                           :context_window window
                           :provider :test
                           :model :model}
                          {:id :test/model :provider :test :model :model})
                      {:id :test/small
                       :context_window 200000
                       :provider :test
                       :model :small}]
            ;; Roles is a collection and this patch merges into the installed models,
            ;; so unassigning a summarizer needs a replace control to survive the merge.
            :roles (misa.replace (or roles {}))}
   :agent {:messages messages :status :ready}
   :usage {:last_request {:input_tokens 190000 :output_tokens 1000}}})

(fn seed! [options]
  (step {:type :probe/seed :value (seed options)}))

(local settings (compaction.settings {}))

;; Above the budget: the conversation is due, so the trigger is what is under test.
(seed! {:window 200000})
(assert (= (compaction.due? settings (seed {:window 200000})) true)
        "the seeded conversation is not due")

;; An unconfigured session queues nothing. No summarizer is assigned, so compaction
;; must stay silent rather than refusing on every turn above the budget.
(local idle (settle {:type :agent/status :status :ready}))
(assert (not (seen? idle :compaction/start)) "compaction started without a summarizer")
(assert (= (notice-text idle) nil) "an unconfigured session reported a refusal")
(assert (= (length (messages-of)) 7) "an unconfigured session changed history")

;; Both documented conditions are required. An assigned summarizer is not enough when
;; the selected model declares no context window, because the budget is then unknown.
(seed! {:roles {:summarizer :test/small}})
(local no-window (settle {:type :agent/status :status :ready}))
(assert (not (seen? no-window :compaction/start))
        "compaction started without a known context window")
(assert (= (notice-text no-window) nil) "an unknown budget reported a refusal")

;; With both conditions met the trigger starts a tool-free summarization request.
(seed! {:window 200000 :roles {:summarizer :test/small}})
(local started (settle {:type :agent/status :status :ready}))
(local start-event (event-of started :compaction/start))
(assert (= (type start-event) :table) "automatic compaction did not start")
(assert (= start-event.automatic true) "the automatic trigger was not marked automatic")
(assert (contains? (notice-text started) "Context limit reached")
        "automatic compaction did not report why it started")
(local request (effect-of started :provider.test))
(assert (= (type request) :table) "compaction did not issue a provider request")
(assert (= request.model :small) "compaction did not use the summarizer model")
(assert (= (length request.tools) 0) "the compaction request carried tools")
(assert (= (length request.messages) 1) "the compaction request carried extra turns")
(assert (contains? request.system_prompt "handoff note")
        "the compaction request lost the handoff prompt")

;; An active request holds draft submission through the lifecycle query.
(local running (read-state))
(assert (= (. running.lifecycle :block_draft) true)
        "an active compaction did not hold draft submission")

;; A summary for the unchanged conversation replaces canonical history with a user
;; handoff and an assistant continuation so the next request still alternates roles.
(step {:type :agent/stream-delta
       :id :compaction-1
       :delta {:type :text :text "Goal: the work so far."}})
(local applied (settle {:type :agent/stream-end :id :compaction-1}))
(local handoff (messages-of))
(assert (= (length handoff) 2) "the applied handoff did not replace the turns")
(assert (= (. handoff 1 :role) :user) "the handoff does not start with a user turn")
(assert (= (. handoff 2 :role) :assistant) "the handoff does not end with an assistant turn")
(assert (contains? (. handoff 1 :content 1 :text) "handoff note")
        "the handoff lost its continuation marker")
(assert (contains? (. handoff 1 :content 1 :text) "Goal: the work so far.")
        "the handoff lost the summary")
(assert (contains? (notice-text applied) "Compacted 7 messages")
        "the applied handoff was not reported")
(read-state)
(assert (= (. observed.db.compaction :active) nil) "bookkeeping was not cleared")
(assert (= (. observed.lifecycle :block_draft) false)
        "a finished compaction still holds draft submission")

;; /compact asks the controller to start, and the controller refuses without an
;; assigned summarizer instead of issuing a request.
(seed! {:window 200000})
(local refused (settle {:type :compaction/request :arguments "focus"}))
(assert (seen? refused :compaction/start) "/compact did not reach the controller")
(assert (= nil (effect-of refused :provider.test))
        "/compact issued a request without a summarizer")
(assert (contains? (notice-text refused) "summarizer")
        "/compact did not name the missing role")
(assert (contains? (notice-text refused) "/role summarizer")
        "/compact did not say how to assign the role")
(read-state)
(assert (= (. observed.db.compaction :active) nil)
        "/compact booked a request it could not start")
(assert (= (length (messages-of)) 7) "/compact changed history without a summarizer")

;; With the role assigned, /compact opens the cancellable progress dialog.
(seed! {:window 200000 :roles {:summarizer :test/small}})
(local dialog (settle {:type :compaction/request :arguments "focus"}))
(local open (event-of dialog :dialog/open))
(assert (= (type open) :table) "/compact did not open the progress dialog")
(assert (= open.id :compaction))
(assert (= open.kind :progress))
(assert (= open.cancellable true))
(assert (= open.correlation :compaction))
(assert (= open.completion :compaction/action))
(assert (= (notice-text dialog) nil) "/compact refused with the role assigned")

(output "compaction cascade contracts passed\n")
