;; Exercise the real transaction pipeline while intercepting native timers so

;; lifecycle assertions do not depend on wall-clock scheduling.

{:setup (fn [context]
          (local setup-fx [])
          (local enabled (not= context.config.animations.enabled false))
          (local steps {})

          (fn step [event check]
            (tset steps (+ (length steps) 1) {: check : event})
            nil)

          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:after (fn [tx]
                                          (local state
                                                 (or tx.db.test_animation
                                                     {:redraws 0
                                                      :starts 0
                                                      :stops 0}))
                                          (set tx.db.test_animation state)
                                          (local keep {})
                                          (each [_ effect (ipairs tx.fx)]
                                            (if (= effect.type :timer/start)
                                                (do
                                                  (assert (and (= effect.id
                                                                  :animation/service)
                                                               (= effect.interval_ms
                                                                  160)))
                                                  (set state.starts
                                                       (+ state.starts 1)))
                                                (= effect.type :timer/stop)
                                                (set state.stops
                                                     (+ state.stops 1))
                                                (do
                                                  (when (and (= effect.type
                                                                :dispatch)
                                                             (= effect.event.type
                                                                :ui/redraw))
                                                    (set state.redraws
                                                         (+ state.redraws 1)))
                                                  (tset keep
                                                        (+ (length keep) 1)
                                                        effect))))
                                          (set tx.fx keep)
                                          tx)
                                 :id :test/animation-effects}})

          (fn counts [db starts stops]
            (assert (= db.test_animation.starts (or (and enabled starts) 0))
                    "unexpected animation timer starts")
            (assert (= db.test_animation.stops (or (and enabled stops) 0))
                    "unexpected animation timer stops")
            nil)

          (step {:type :test/idle} (fn [db] (counts db 0 0) nil))
          (step {:status :thinking :type :agent/status}
                (fn [db]
                  (counts db 1 0)
                  (assert (= (misa.animation_frame db :status)
                             (or (and enabled "·") "…")))
                  nil))
          (step {:status :streaming :type :agent/status}
                (fn [db] (counts db 1 0) nil))
          (step {:id :unrelated :type :animations/tick}
                (fn [db] (assert (= db.animations.ticks.status 0)) nil))
          (step {:id :animation/service :type :animations/tick}
                (fn [db]
                  (assert (= (misa.animation_frame db :status)
                             (or (and enabled "•") "…")))
                  (assert (= db.test_animation.redraws (or (and enabled 1) 0)))
                  nil))
          (step {:role :other :type :animations/start}
                (fn [db] (counts db 1 0) nil))
          (step {:status :ready :type :agent/status}
                (fn [db] (counts db 1 0) nil))
          (step {:role :other :type :animations/stop}
                (fn [db] (counts db 1 1) nil))
          (step {:id :animation/service :type :animations/tick}
                (fn [db]
                  (assert (= db.test_animation.redraws (or (and enabled 1) 0))
                          "stale tick redrew idle UI")
                  nil))
          (step {:animation :static :type :animations/swap})
          (step {:status :thinking :type :agent/status}
                (fn [db] (counts db 1 1) nil))
          (step {:animation :default :type :animations/swap}
                (fn [db] (counts db 2 1) nil))
          (step {:animation :static :role :status :type :animations/swap}
                (fn [db] (counts db 2 2) nil))
          (step {:data {:active :default :roles {:status :default}}
                 :found true
                 :type :animations/loaded}
                (fn [db] (counts db 3 2) nil))
          (step {:status :ready :type :agent/status}
                (fn [db] (counts db 3 3) nil))
          (step {:status :thinking :type :agent/status}
                (fn [db] (counts db 4 3)
                  (assert (= db.animations.ticks.status 0)
                          "new work did not reset animation phase")
                  nil))
          (step {:status :ready :type :agent/status}
                (fn [db] (counts db 4 4) nil))
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:event {:index 1
                                                   :type :test/animation-step}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/animation-step
                         :handler (fn [db event]
                                    (local current (. steps event.index))
                                    (if (not current)
                                        {:fx [{:lines [{:spans [{:text :animations}]}]
                                               :type :view/commit}
                                              {:type :app/quit}]}
                                        {:fx [{:event current.event
                                               :type :dispatch}
                                              {:event {:index event.index
                                                       :type :test/animation-check}
                                               :type :dispatch}]}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/animation-check
                         :handler (fn [db event]
                                    (local check (. steps event.index :check))
                                    (when check (check db))
                                    {:fx [{:event {:index (+ event.index 1)
                                                   :type :test/animation-step}
                                           :type :dispatch}]})})
          nil
          {:fx setup-fx})}
