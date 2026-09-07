;; Whole transcript model preparation, component rendering, and themed output.
;; CPU timings exclude oracle serialization, setup, and synthetic input creation.
;; This is not native presentation, event dispatch, or time-to-first-frame.
(local fennel (require :fennel))
(local clock os.clock)
(local output print)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
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
    (fn run [record]
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
    (run true)
    ;; Six unchanged repetitions establish the oracle and warm both paths.
    (for [_ 1 6] (run false))
    (local renders [])
    (local updates [])
    (for [iteration 1 10]
      (local (render-time update-time) (run false))
      (table.insert renders render-time)
      (table.insert updates update-time)
      (output (string.format "sample,%d,%s,%d,render_s=%.9f,update_s=%.9f"
                             count mode iteration render-time update-time)))
    (table.sort renders)
    (table.sort updates)
    (output (string.format "summary,%d,%s,frames=8,render_median_s=%.9f,render_best_s=%.9f,update_median_s=%.9f,update_best_s=%.9f"
                           count mode (/ (+ (. renders 5) (. renders 6)) 2) (. renders 1)
                           (/ (+ (. updates 5) (. updates 6)) 2) (. updates 1)))))
