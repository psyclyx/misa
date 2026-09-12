;; Lua-side projection phases for the native transcript fixture, by transcript
;; size and frame kind. This attributes model dispatch and presentation
;; separately, so a transcript-sized cost is visible instead of averaged into a
;; wall-clock frame that the native presenter dominates.
;;
;; tools/fennel benchmarks/projection-phases.fnl [blocks] [iterations]
;;
;; Frames are dispatched through the real transaction pipeline with native
;; effects discarded, matching `benchmarks/transcript-profile.fnl`. Nothing here
;; times native presentation, terminal output, or scheduling.
(local fennel (require :fennel))
(local os-clock (. os :clock))
(local output print)
(local blocks (tonumber (or (. arg 1) "300")))
(local iterations (tonumber (or (. arg 2) "80")))
(assert (and blocks (>= blocks 1)) "blocks must be a positive integer")
(assert (and iterations (>= iterations 1)) "iterations must be a positive integer")
(require :tests.application)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local application ((require :benchmarks.application) :native-transcript
                     {:blocks blocks}))
(misa._install application.definitions
               {:argv [] :config application.config :host {:executable :misa}})
(local terminal {:columns 100 :lines 32 :interactive true :images false})
(local clock {:wall_ms 0 :monotonic_ms 0})
(var sampling false)
(var best-model math.huge)
(var best-presentation math.huge)

(fn milliseconds [] (* 1000 (os-clock)))

(fn note [name start]
  (when sampling
    (if (= name :model)
        (set best-model (math.min best-model (- (milliseconds) start)))
        (set best-presentation (math.min best-presentation
                                         (- (milliseconds) start))))))

(fn project []
  (local start (milliseconds))
  (misa._project terminal clock)
  (misa._commit_projection)
  (note :presentation start))

(fn frame [event]
  "Dispatch one event, commit it, and project the committed state."
  (local queue [event])
  (var index 1)
  (local model-start (milliseconds))
  (while (<= index (length queue))
    (local effects (misa._dispatch (. queue index) terminal clock))
    (misa._commit)
    (each [_ effect (ipairs effects)]
      (when (= effect.type :dispatch) (table.insert queue effect.event)))
    (set index (+ index 1)))
  (note :model model-start)
  (project))

(fn measure [kind]
  (set (best-model best-presentation) (values math.huge math.huge))
  (set sampling true)
  (for [_ 1 iterations]
    ;; A repeated projection without a dispatch isolates the presentation, which
    ;; is the cache-hit path an unchanged transcript takes inside a redraw frame.
    (if (= kind :static)
        (project)
        (frame {:type :bench/frame :stream (= kind :delta)})))
  (set sampling false)
  (values best-model best-presentation))

(output "blocks,frame,iterations,best_model_ms,best_presentation_ms,best_total_ms")
(frame {:type :app/start})
(frame {:type :bench/frame :stream true})
(frame {:type :bench/frame :stream true})
(each [_ kind (ipairs [:static :redraw :delta])]
  (frame {:type :bench/frame :stream true})
  (local (model-ms presentation-ms) (measure kind))
  (local unmeasured (= model-ms math.huge))
  (output (string.format "%d,%s,%d,%s,%.4f,%.4f" blocks kind iterations
                         (if unmeasured "" (string.format "%.4f" model-ms))
                         presentation-ms
                         (+ (if unmeasured 0 model-ms) presentation-ms))))
