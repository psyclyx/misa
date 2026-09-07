(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(each [_ name (ipairs [:json :layout :markdown])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) {:config {}}))
(local specs ((. (fennel.dofile :extensions/syntax.fnl) :setup) {:config {}}))
(local handlers {})
(var policy nil)
(each [_ spec (ipairs specs.fx)]
  (when (= spec.type :register/event) (tset handlers spec.name spec.handler))
  (when (= spec.type :register/interceptor) (set policy spec.value)))
(misa._setup_effects specs)
(fn unchanged [db call]
  (local before (misa.json.encode db))
  (local result (call))
  (assert (= before (misa.json.encode db)) "syntax transition mutated prior state")
  result)
(fn event [db input]
  (local incoming {: db :event input})
  (local tx (unchanged incoming #(policy.before incoming)))
  (local result (unchanged tx.db #((. handlers input.type) tx.db input)))
  (assert (not (and result result.db)))
  (values (misa.patch tx.db (or (and result result.patch) {})) (or (and result result.fx) [])))
(fn source [db text]
  (local incoming {: db :event {:type :transcript/block-delta
                                                      :response_id :reply :block_id :body}
                                         :cofx {:terminal {:interactive true}} :fx []})
  (var tx (unchanged incoming #(policy.before incoming)))
  (set tx.db (misa.patch tx.db {:messages {:blocks (misa.replace [{:id :body :kind :assistant
                                                                 :response_id :reply : text}])
                                         :by_response {:reply 1}
                                         :responses [{:block_start 1 :block_count 1}]}}))
  (set tx (unchanged tx #(policy.after tx)))
  (values tx.db tx.fx))
(fn initial [] (event {} {:type :app/start}))
(local text "```lua\nlocal x=1\n```")
(local (first requests) (source (initial) text))
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
(local projection (misa.syntax_projection (misa.syntax_projections retained) model))
(local expected (fennel.view projection))
(assert (= projection (misa.syntax_projection (misa.syntax_projections (misa.patch retained {:unrelated true})) model))
        "unrelated state invalidated the syntax subscription")
(local later (source retained "```lua\nlocal x=99\n```"))
(local reset-later (event later {:type :transcript/reset}))
(event reset-later {:type :syntax/completed :id :unrelated})
(assert (= expected (fennel.view (misa.syntax_projection (misa.syntax_projections retained) model)))
        "later syntax work invalidated a retained state's projection")
;; Collection updates preserve other documents and request identities. No-op
;; events return the transaction itself rather than rebuilding a syntax snapshot.
(local many-blocks (fcollect [i 1 300]
                     {:id (tostring i) :response_id :many :kind :assistant : text}))
(local many-input {:db (misa.patch (initial)
                                 {:messages {:blocks many-blocks :by_response {:many 1}
                                             :responses [{:block_start 1 :block_count 300}]}})
                   :event {:type :seed} :cofx {:terminal {:interactive true}}
                   :syntax_count 0 :fx [{:type :sentinel}]})
(local seeded (unchanged many-input #(policy.after many-input)))
(assert (= (length seeded.fx) 301))
(assert (= seeded.db.syntax.next_id 300))
(for [i 1 300]
  (local request (. seeded.fx (+ i 1)))
  (assert (= request.id (.. :syntax/ i)))
  (assert (= (. seeded.db.syntax.pending request.id :key) (.. "4:many" i))))
(assert (= (. seeded.fx 1) (. many-input.fx 1)))
(assert (= (next many-input.db.syntax.documents) nil))
(assert (= (policy.after seeded) seeded) "unchanged syntax rebuilt transaction")
(local delta-input (misa.patch (policy.before seeded)
                              {:event (misa.replace {:type :transcript/block-delta
                                                     :response_id :many :block_id :300})
                               :db {:messages {:blocks (misa.replace
                                                         (icollect [i block (ipairs many-blocks)]
                                                           (if (= i 300)
                                                               (misa.patch block {:text "```lua\nlocal x=2\n```"})
                                                               block)))}}
                               :fx (misa.replace [])}))
(local many-changed (unchanged delta-input #(policy.after delta-input)))
(assert (= (length many-changed.fx) 0) "pending slot spawned a concurrent request")
(for [i 1 299]
  (local key (.. "4:many" i))
  (assert (= (. many-changed.db.syntax.documents key) (. seeded.db.syntax.documents key))))
(assert (= many-changed.db.syntax.pending seeded.db.syntax.pending))
(local (many-completed followup)
       (event many-changed.db {:type :syntax/completed :id (. seeded.fx 301 :id)
                               :ok true :data []}))
(assert (= (length followup) 1))
(assert (= (. followup 1 :source) "local x=2"))
(for [i 1 299]
  (local key (.. "4:many" i))
  (assert (= (. many-completed.syntax.documents key) (. seeded.db.syntax.documents key))))
(assert (= (length seeded.fx) 301))
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
