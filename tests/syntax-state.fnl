(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local misa _G.misa)
(each [_ name (ipairs [:json :layout :markdown])]
  (app.define ((fennel.dofile (.. :extensions/ name :.fnl)) {:config {}})))
(local specs ((fennel.dofile :extensions/syntax.fnl) {:config {}}))
(local handlers {})
(each [_ spec (pairs (. specs :events))]
  (tset handlers spec.event spec.handler))
(app.define specs)
(app.define (definitions :test [{:catalog :services :id :transcript.blocks :value (fn [db response-id block-id]
                                   (icollect [_ block (ipairs (or (and db.messages db.messages.blocks) []))]
                                     (when (and (or (not response-id) (= block.response_id response-id))
                                                (or (not block-id) (= block.id block-id))) block)))}]))
(app.install)
(fn unchanged [db call]
  (local before (misa.json.encode db))
  (local result (call))
  (assert (= before (misa.json.encode db)) "syntax transition mutated prior state")
  result)
(fn event [db input]
  (local result (unchanged db #((. handlers input.type) db input {:terminal {:interactive true}})))
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(fn source [db text]
  (local next (misa.patch db {:messages {:blocks (misa.replace [{:id :body :kind :assistant
                                                               :response_id :reply : text}])}}))
  (event next {:type :transcript/updated :response_id :reply :block_id :body}))
(fn initial [] (event {} {:type :app/start}))
(local text "```lua\nlocal x=1\n```")
(local (first requests) (source (initial) text))
(assert (= ((. handlers :transcript/updated) first {:type :transcript/updated}
            {:terminal {:interactive false}}) nil) "headless transcript requested highlighting")
(local disabled ((fennel.dofile :extensions/syntax.fnl)
                 {:config {:messages {:markdown false}}}))
(each [_ spec (pairs (. disabled :events))]
  (when (= spec.event :transcript/updated)
    (assert (= (spec.handler first {:type :transcript/updated}
                            {:terminal {:interactive true}}) nil)
            "disabled Markdown requested highlighting")))
(local id (. requests 1 :id))
(local (_ entry) (next first.syntax.documents))
(local start (. entry.slots 1 :start))
(local (shifted extra) (source first (.. "intro\n\n" text)))
(assert (= (length extra) 0) "unchanged code requested highlighting again")
(assert (= (. entry.slots 1 :start) start))
(local (_ shifted-entry) (next shifted.syntax.documents))
(assert (> (. shifted-entry.slots 1 :start) start))
(local changed (source shifted "```lua\nlocal x=2\n```"))
(local (completed next-requests) (event changed {:type :syntax/completed : id :ok true :data []}))
(assert (= (length next-requests) 1))
(assert (= (. next-requests 1 :source) "local x=2"))
(local reset (event completed {:type :transcript/reset}))
(assert (= (next reset.syntax.pending) nil))
(assert (= (next reset.syntax.documents) nil))
(assert (= (event reset {:type :syntax/completed :id (. next-requests 1 :id) :ok true :data []}) reset))
(local failure
       (G.for_all (G.vector (G.elements [:first :second :complete :reset]))
                  (fn [actions]
                    (var db (initial))
                    (each [_ action (ipairs actions)]
                      (if (= action :reset) (set db (event db {:type :transcript/reset}))
                          (= action :complete)
                          (let [pending-id (next db.syntax.pending)]
                            (when pending-id
                              (set db (event db {:type :syntax/completed :id pending-id :ok true :data []}))))
                          (set db (source db (if (= action :first) text "```lua\nlocal x=2\n```"))))
                      (var count 0)
                      (each [_ _ (pairs db.syntax.pending)] (set count (+ count 1)))
                      (assert (<= count 1) "streaming created concurrent requests for one slot")))
                  {:cases 1000 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
;; A retained state must remain renderable after later updates and resets.
(local (retained-pending retained-fx) (source (initial) text))
(local retained (event retained-pending {:type :syntax/completed :id (. retained-fx 1 :id)
                                       :ok true :data [{:start_byte 0 :end_byte 5 :capture :keyword}]}))
(local model (. retained.messages.blocks 1))
(local projection (misa.syntax.for-model (misa.syntax.all retained) model))
(local expected (fennel.view projection))
(assert (= projection (misa.syntax.for-model (misa.syntax.all (misa.patch retained {:unrelated true})) model))
        "unrelated state invalidated the syntax subscription")
(local later (source retained "```lua\nlocal x=99\n```"))
(local reset-later (event later {:type :transcript/reset}))
(event reset-later {:type :syntax/completed :id :unrelated})
(assert (= expected (fennel.view (misa.syntax.for-model (misa.syntax.all retained) model)))
        "later syntax work invalidated a retained state's projection")
;; Collection updates preserve other documents and request identities. No-op
;; notifications return no patch rather than rebuilding a syntax snapshot.
(local many-blocks (fcollect [i 1 300]
                     {:id (tostring i) :response_id :many :kind :assistant : text}))
(local many-input (misa.patch (initial) {:messages {:blocks many-blocks}}))
(local (seeded seeded-fx) (event many-input {:type :transcript/updated}))
(assert (= (length seeded-fx) 300))
(assert (= seeded.syntax.next_id 300))
(for [i 1 300]
  (local request (. seeded-fx i))
  (assert (= request.id (.. :syntax/ i)))
  (assert (= (. seeded.syntax.pending request.id :key) (.. "4:many" i))))
(assert (= (next many-input.syntax.documents) nil))
(assert (= (event seeded {:type :transcript/updated}) seeded) "unchanged syntax rebuilt state")
(assert (= (event seeded {:type :transcript/updated :response_id :missing}) seeded))
(local delta-input (misa.patch seeded
                              {:messages {:blocks (misa.replace
                                                         (icollect [i block (ipairs many-blocks)]
                                                           (if (= i 300)
                                                               (misa.patch block {:text "```lua\nlocal x=2\n```"})
                                                               block)))}}))
(local (many-changed changed-fx) (event delta-input {:type :transcript/updated :response_id :many :block_id :300}))
(assert (= (length changed-fx) 0) "pending slot spawned a concurrent request")
(for [i 1 299]
  (local key (.. "4:many" i))
  (assert (= (. many-changed.syntax.documents key) (. seeded.syntax.documents key))))
(assert (= many-changed.syntax.pending seeded.syntax.pending))
(local (many-completed followup)
       (event many-changed {:type :syntax/completed :id (. seeded-fx 300 :id)
                               :ok true :data []}))
(assert (= (length followup) 1))
(assert (= (. followup 1 :source) "local x=2"))
(for [i 1 299]
  (local key (.. "4:many" i))
  (assert (= (. many-completed.syntax.documents key) (. seeded.syntax.documents key))))
(assert (= (length seeded-fx) 300))
;; Multiple slots allocate independent requests; out-of-order completions must
;; leave the other slot and its pending request intact.
(local (multi multi-fx) (source (initial) (.. text "\n\n```python\nx=1\n```")))
(assert (= (length multi-fx) 2))
(assert (not= (. multi-fx 1 :id) (. multi-fx 2 :id)))
(local second-done (event multi {:type :syntax/completed :id (. multi-fx 2 :id)
                                 :ok true :data []}))
(assert (. second-done.syntax.pending (. multi-fx 1 :id)))
(assert (= (. second-done.syntax.pending (. multi-fx 2 :id)) nil))
(assert (= (. second-done.syntax.documents "5:replybody" :slots 1)
           (. multi.syntax.documents "5:replybody" :slots 1)))
(local all-done (event second-done {:type :syntax/completed :id (. multi-fx 1 :id)
                                   :ok true :data []}))
(assert (= (next all-done.syntax.pending) nil))
(each [_ slot (ipairs (. all-done.syntax.documents "5:replybody" :slots))]
  (assert slot.done)
  (assert (= slot.request nil)))
(output "syntax state properties passed\n")
