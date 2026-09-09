(local definitions (require :tests.declarations))

;; Exercise the real transaction pipeline while intercepting native timers so

;; lifecycle assertions do not depend on wall-clock scheduling.

(fn [context]
          (local declarations [])
          (local enabled (not= context.config.animations.enabled false))
          (local steps {})

          (fn step [event check]
            (tset steps (+ (length steps) 1) {: check : event})
            nil)

          ;; Observe native effect boundaries without running real timers.
          (local observed {:starts 0 :stops 0 :redraws 0})
          (table.insert declarations
                        {:catalog :effects :id :timer/start :value (fn [effect]
                                    (assert (and (= effect.id :animation/service)
                                                 (= effect.interval_ms 160)))
                                    (set observed.starts (+ observed.starts 1))
                                    {})})
          (table.insert declarations
                        {:catalog :effects :id :timer/stop :value (fn []
                                    (set observed.stops (+ observed.stops 1))
                                    {})})
          (table.insert declarations
                        {:catalog :events  :value {:event :ui/redraw :handler (fn []
                                    (set observed.redraws (+ observed.redraws 1))
                                    nil)}})
          (table.insert declarations
                        {:catalog :components :id :default.test-animation :value {:render (fn []
                                           {:lines [{:surface :plain
                                                     :spans [{:text :x
                                                              :style :value
                                                              :animation {:id :style
                                                                          :interval_ms 100
                                                                          :frames [{:style :bold}
                                                                                   {}]}}]}]})}})

          (fn check-projection [db busy]
            (local spans (. (misa.status.model db {:columns 80}) 1 :spans))
            (assert (= (. spans 3 :text) (or (and busy :thinking) :ready)))
            (if busy
                (do
                  (assert (= (. spans 4 :text) (or (and enabled "·") "…")))
                  (assert (= (not= (. spans 4 :animation) nil) enabled))
                  (when enabled
                    (assert (= (. spans 4 :animation :id) :animation/status))))
                (each [_ span (ipairs spans)] (assert (= span.animation nil))))
            nil)

          (fn check-theme-layout [db]
            (local projected (. (misa.components.render db :test-animation {} {})
                                :lines 1 :spans 1))
            (local override (. projected.animation.frames 1 :style))
            (assert (= (type override) :table))
            (assert (= (. projected.animation.frames 2 :style) nil)
                    "unspecified frame style should inherit its span")
            (each [key value (pairs (misa.themes.style db :bold))]
              (assert (= (tostring (. override key)) (tostring value))))
            (local source
                   {:text :xx
                    :animation {:id :clip
                                :interval_ms 100
                                :frames [{:text :xx} {:text :yy}]}})
            (assert (not= (. (misa.layout.flow-spans [source] 2) 1 :spans 1
                             :animation) nil))
            (assert (= (. (misa.layout.flow-spans [source] 1) 1 :spans 1
                          :animation) nil))
            nil)

          (fn counts [db starts stops]
            (assert (= observed.starts (or (and enabled starts) 0))
                    "unexpected animation timer starts")
            (assert (= observed.stops (or (and enabled stops) 0))
                    "unexpected animation timer stops")
            nil)

          (step {:type :test/idle} (fn [db] (counts db 0 0)
                                     (check-projection db false)
                                     (check-theme-layout db)
                                     nil))
          (step {:status :thinking :type :agent/status}
                (fn [db]
                  (counts db 0 0)
                  (check-projection db true)
                  (local span (misa.animations.span db :status))
                  (assert (= span.text (or (and enabled "·") "…")))
                  (assert (= (not= span.animation nil) enabled))
                  (when enabled
                    (assert (= span.animation.id :animation/status))
                    (assert (= span.animation.interval_ms 160))
                    (assert (= (. span.animation.frames 2 :text) "•"))
                    (local phased
                           (misa.animations.span db :status
                                                {:id :shared
                                                 :phase 1
                                                 :style :accent}))
                    (assert (= phased.text "•"))
                    (assert (= phased.animation.phase 1))
                    (assert (= phased.animation.id :shared))
                    (assert (= phased.style :accent)))
                  (assert (= (misa.animations.frame db :status)
                             (or (and enabled "·") "…")))
                  nil))
          (step {:status :streaming :type :agent/status}
                (fn [db] (counts db 0 0) nil))
          (step {:role :status :type :animations/start}
                (fn [db] (counts db 1 0) nil))
          (step {:id :unrelated :type :animations/tick}
                (fn [db] (assert (= db.animations.ticks.status 0)) nil))
          (step {:id :animation/service :type :animations/tick}
                (fn [db]
                  (assert (= (misa.animations.frame db :status)
                             (or (and enabled "•") "…")))
                  (assert (= observed.redraws (or (and enabled 1) 0)))
                  nil))
          (step {:role :other :type :animations/start}
                (fn [db] (counts db 1 0) nil))
          (step {:status :ready :type :agent/status}
                (fn [db] (counts db 1 0) (check-projection db false) nil))
          (step {:role :status :type :animations/stop}
                (fn [db] (counts db 1 0) nil))
          (step {:role :other :type :animations/stop}
                (fn [db] (counts db 1 1) nil))
          (step {:id :animation/service :type :animations/tick}
                (fn [db]
                  (assert (= observed.redraws (or (and enabled 1) 0))
                          "stale tick redrew idle UI")
                  nil))
          (step {:animation :static :type :animations/swap})
          (step {:status :thinking :type :agent/status}
                (fn [db] (counts db 1 1) nil))
          (step {:role :status :type :animations/start}
                (fn [db]
                  (counts db 1 1)
                  (assert (= (. (misa.animations.span db :status) :animation)
                             nil))
                  nil))
          (step {:animation :default :type :animations/swap}
                (fn [db] (counts db 2 1) nil))
          (step {:animation :static :role :status :type :animations/swap}
                (fn [db] (counts db 2 2) nil))
          (step {:data {:active :default :roles {:status :default}}
                 :found true
                 :type :animations/loaded}
                (fn [db] (counts db 3 2) nil))
          (step {:status :ready :type :agent/status}
                (fn [db] (counts db 3 2) nil))
          (step {:role :status :type :animations/stop}
                (fn [db] (counts db 3 3) nil))
          (step {:role :status :type :animations/start}
                (fn [db] (counts db 4 3)
                  (assert (= db.animations.ticks.status 0)
                          "new work did not reset animation phase")
                  nil))
          (step {:role :status :type :animations/stop}
                (fn [db] (counts db 4 4) nil))
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:event {:index 1
                                                   :type :test/animation-step}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/animation-step :handler (fn [db event]
                                    (local current (. steps event.index))
                                    (if (not current)
                                        {:fx [{:lines [{:spans [{:text :animations}]}]
                                               :type :view/commit}
                                              {:type :app/quit}]}
                                        {:fx [{:event current.event
                                               :type :dispatch}
                                              {:event {:index event.index
                                                       :type :test/animation-settle}
                                               :type :dispatch}]}))}})
          ;; Let queued redraw observations complete before checking the step.
          (table.insert declarations
                        {:catalog :events  :value {:event :test/animation-settle :handler (fn [_ event]
                                    {:fx [{:type :dispatch
                                           :event {:type :test/animation-check :index event.index}}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/animation-check :handler (fn [db event]
                                    (local check (. steps event.index :check))
                                    (when check (check db))
                                    {:fx [{:event {:index (+ event.index 1)
                                                   :type :test/animation-step}
                                           :type :dispatch}]})}})
          nil
          (definitions.collect :tests.animations declarations {}))
