;; Whole transcript model preparation, component rendering, and themed output.
;; CPU timings exclude oracle serialization, setup, and synthetic input creation.
;; This is not native presentation, event dispatch, or time-to-first-frame.
(local fennel (require :fennel))
(local clock os.clock)
(local output print)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local candidate {:patch misa.patch :replace misa.replace :delete misa.delete})
(local baseline-path (. arg 1))
(local baseline (if baseline-path ((fennel.dofile baseline-path) misa.json_null) candidate))
(local context {:config {}})
(each [_ name (ipairs [:json :layout :markdown :themes :theme/default :components
                       :component/markdown :component/message])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))
(local specs ((. (fennel.dofile :extensions/messages.fnl) :setup) context))
(misa._setup_effects specs)
(var delta nil)
(each [_ spec (ipairs specs.fx)]
  (when (= spec.name :transcript/block-delta) (set delta spec.handler)))
(assert delta)
(local render-context {:columns 80 :interactive true})
(local source (string.rep "A **bold** [link](https://example.test) with 界 and é.\n\n" 3))
(local fragment "stream **bold** [link](https://example.test)\n\n")
(local cofx {:clock {:wall_ms 1000 :monotonic_ms 1000} :terminal {:interactive true}})

(each [_ count (ipairs [1 16 300])]
  (local blocks (fcollect [i 1 count]
                  {:id (tostring i) :response_id :reply :kind :assistant
                   :text source :started_wall_ms 0}))
  (tset blocks count {:id (tostring count) :response_id :reply :kind :assistant
                      :streaming true :chunks [source] :byte_count (length source)
                      :started_wall_ms 0})
  (local initial {:components {:roles {}} :themes {:active :default}
                  :messages {:blocks blocks :by_response {:reply 1}
                             :responses [{:id :reply :role :assistant :block_start 1
                                          :block_count count :status :streaming}]}})
  (local original (misa.json.encode initial))
  (local event {:type :transcript/block-delta :response_id :reply
                :block_id (tostring count) :text fragment})
  (each [_ mode (ipairs [:redraw :stream])]
    (local oracle [])
    (fn run [record api]
      (set misa.patch api.patch)
      (set misa.replace api.replace)
      (set misa.delete api.delete)
      (var db initial)
      (var (render-time update-time) (values 0 0))
      (for [frame 1 8]
        (when (= mode :stream)
          (local start (clock))
          (local result (delta db event cofx))
          (set db (misa.patch db (or (and result result.patch) {})))
          (set update-time (+ update-time (- (clock) start))))
        (local start (clock))
        (local lines (misa.transcript_projection db render-context))
        (set render-time (+ render-time (- (clock) start)))
        ;; Compare complete output records, including styles and link metadata.
        (local encoded (misa.json.encode lines))
        (if record (tset oracle frame encoded)
            (assert (= encoded (. oracle frame)) "transcript output changed between identical runs")))
      (assert (= original (misa.json.encode initial)) "projection or reducer mutated initial state")
      (when (= mode :stream)
        (assert (= (table.concat (. db.messages.blocks count :chunks))
                   (.. source (string.rep fragment 8))) "stream content was lost")
        (for [i 1 (- count 1)]
          (assert (= (. initial.messages.blocks i) (. db.messages.blocks i))
                  "stream update replaced an unrelated block")))
      (values render-time update-time))
    (run true baseline)
    ;; Six unchanged repetitions establish the oracle and warm both paths.
    (for [_ 1 6] (run false baseline) (when baseline-path (run false candidate)))
    (local samples {:baseline {:renders [] :updates [] :totals []}
                    :candidate {:renders [] :updates [] :totals []}})
    (for [iteration 1 10]
      (each [_ variant (ipairs (if (not baseline-path) [:baseline]
                                  (= (% iteration 2) 0) [:candidate :baseline] [:baseline :candidate]))]
        (local (render-time update-time) (run false (if (= variant :baseline) baseline candidate)))
        (table.insert (. samples variant :renders) render-time)
        (table.insert (. samples variant :updates) update-time)
        (table.insert (. samples variant :totals) (+ render-time update-time))))
    (each [variant sample (pairs samples)]
      (when (> (length sample.renders) 0)
        (local renders sample.renders)
        (local updates sample.updates)
        (local totals sample.totals)
        (table.sort renders)
        (table.sort updates)
        (table.sort totals)
        (output (string.format "summary,%d,%s,%s,frames=8,render_median_s=%.9f,render_best_s=%.9f,update_median_s=%.9f,update_best_s=%.9f,total_median_s=%.9f,total_best_s=%.9f"
                               count mode variant (/ (+ (. renders 5) (. renders 6)) 2) (. renders 1)
                               (/ (+ (. updates 5) (. updates 6)) 2) (. updates 1)
                               (/ (+ (. totals 5) (. totals 6)) 2) (. totals 1)))))))
