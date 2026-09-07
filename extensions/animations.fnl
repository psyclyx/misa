;; Animation registry with clock-driven spans and optional explicit timer events.
;; Registrations are immutable once

;; app/start begins; selections, role ticks, and running timers are transactional.

{:setup (fn [context]
          (local setup-fx [])
          (local entries {})
          (var config (or (and (= (type context.config) :table)
                               context.config.animations)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local configured (or (and (= (type config.default) :string)
                                     config.default)
                                :default))
          (local configured-roles (or (and (= (type config.roles) :table)
                                           config.roles)
                                      {}))
          (local interval-ms (or config.interval_ms 160))
          (assert (or (= config.enabled nil) (= (type config.enabled) :boolean))
                  "animations.enabled must be a boolean")
          (local enabled (not= config.enabled false))
          (assert (and (and (and (= (type interval-ms) :number)
                                 (>= interval-ms 10))
                            (<= interval-ms 60000))
                       (= (% interval-ms 1) 0))
                  "animations.interval_ms must be an integer from 10 through 60000")
          (table.insert setup-fx
                        {:type :register/setup-effect
                         :name :register/animation
                         :handler (fn [effect]
                                    (let [id effect.id
                                          animation effect.value]
                                      (assert (and (and (= (type id) :string)
                                                        (not= id ""))
                                                   (= (type animation) :table))
                                              "invalid animation")
                                      (assert (and (= (type animation.frames)
                                                      :table)
                                                   (> (length animation.frames)
                                                      0))
                                              "animation frames must be nonempty")
                                      (each [_ frame (ipairs animation.frames)]
                                        (assert (= (type frame) :string)
                                                "animation frames must be strings"))
                                      (assert (or (= animation.still nil)
                                                  (= (type animation.still)
                                                     :string))
                                              "animation still frame must be a string")
                                      (assert (= (. entries id) nil)
                                              (.. "duplicate animation: " id))
                                      (tset entries id animation)
                                      nil))})

          (fn animation-id [db role]
            (local state
                   (assert db.animations "animation state is not initialized"))
            (or (and role (. state.roles role)) state.active))

          (table.insert setup-fx
                        {:type :register/service
                         :name :animation
                         :value (fn [db role]
                                  (assert (= (type db) :table)
                                          "animation resolution requires initialized db")
                                  (local id (animation-id db role))
                                  (assert (. entries id)
                                          (.. "unknown animation: "
                                              (tostring id))))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :animation_frame
                         :value (fn [db role tick]
                                  (local animation (misa.animation db role))
                                  (local frames animation.frames)
                                  (if (not enabled)
                                      (or animation.still (. frames 1))
                                      (do
                                        (var current tick)
                                        (when (= current nil)
                                          (set current
                                               (or (. (or db.animations.ticks
                                                          {})
                                                      (or role :default))
                                                   0)))
                                        (. frames
                                           (+ (% current (length frames)) 1)))))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :animation_span
                         :value (fn [db role options]
                                  (local animation (misa.animation db role))
                                  (local frames animation.frames)
                                  (local opts (or options {}))
                                  (local phase (or opts.phase 0))
                                  (assert (and (and (= (type phase) :number)
                                                    (= (% phase 1) 0))
                                               (and (>= phase 0)
                                                    (< phase (length frames))))
                                          "animation phase must name a frame")
                                  (local span
                                         {:text (or (and (not enabled)
                                                         (or animation.still
                                                             (. frames 1)))
                                                    (. frames (+ phase 1)))
                                          :style opts.style})
                                  (var distinct false)
                                  (each [_ frame (ipairs frames)]
                                    (when (not= frame (. frames 1))
                                      (set distinct true)))
                                  (when (and enabled distinct)
                                    (local id
                                           (or opts.id
                                               (.. :animation/
                                                   (or role :default))))
                                    (assert (and (and (= (type id) :string)
                                                      (> (length id) 0))
                                                 (<= (length id) 256))
                                            "animation ID must contain 1 through 256 bytes")
                                    (assert (<= (length frames) 64)
                                            "clock animation supports at most 64 frames")
                                    (local projected {})
                                    (each [_ frame (ipairs frames)]
                                      (table.insert projected {:text frame}))
                                    (set span.animation
                                         {: id
                                          :interval_ms interval-ms
                                          : phase
                                          :frames projected}))
                                  span)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :swap_animation
                         :value (fn [db id role]
                                  (assert (. entries id)
                                          (.. "unknown animation: "
                                              (tostring id)))
                                  (misa.patch db
                                              {:animations (if role {:roles {role id}}
                                                               {:active id})}))})
          (local timer-id :animation/service)

          (fn moving [db role]
            (and enabled (> (length (. (misa.animation db role) :frames)) 1)))

          (fn reconcile-timer [db]
            (var needed false)
            (each [role (pairs db.animations.running)]
              (when (moving db role) (set needed true) (lua :break)))
            {:patch {:animations
                     (misa.replace (misa.patch db.animations {:timer_running needed}))}
             :fx (if (= needed (= db.animations.timer_running true)) {}
                  (or (and needed
                           [{:completion :animations/tick
                             :id timer-id
                             :interval_ms interval-ms
                             :type :timer/start}])
                      [{:id timer-id :type :timer/stop}]))})

          (fn start-role [db role]
            (local running db.animations.running)
            (when (not (. running role))
              (reconcile-timer
                (misa.patch db {:animations {:running {role true}
                                             :ticks {role 0}}}))))

          (fn stop-role [db role]
            (local running db.animations.running)
            (when (. running role)
              (reconcile-timer
                (misa.patch db {:animations {:running {role misa.delete}}}))))

          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (local initial
                                           (when (not db.animations)
                                             {:active configured :running {} :ticks {}
                                              :roles (collect [role id (pairs configured-roles)]
                                                       (do (assert (and (= (type role) :string)
                                                                    (not= role "")
                                                                    (= (type id) :string)
                                                                    (. entries id))
                                                               "invalid configured animation role")
                                                           (values role id)))}))
                                    (local active (. (or db.animations initial) :active))
                                    (assert (. entries active)
                                            (.. "unknown configured animation: "
                                                (tostring active)))
                                    {:patch (when initial {:animations (misa.replace initial)})
                                     :fx (when (not= config.persist false)
                                          [{:completion :animations/loaded
                                               :namespace :ui.animation
                                               :type :state/load}])})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :animations/loaded
                         :handler (fn [db event]
                                    (if (or (= event.found false)
                                            (= event.data misa.json_null))
                                        nil
                                        (do
                                          (assert (and (= (type event.data)
                                                          :table)
                                                       (= (type event.data.active)
                                                          :string))
                                                  "invalid persisted animation")
                                          (local roles {})
                                          (each [role id (pairs (or (and (= (type event.data.roles)
                                                                            :table)
                                                                         event.data.roles)
                                                                    {}))]
                                            (when (and (= (type role) :string)
                                                       (. entries id))
                                              (tset roles role id)))
                                          (reconcile-timer
                                            (misa.patch db
                                                        {:animations {: roles
                                                                      :active (when (. entries event.data.active)
                                                                                event.data.active)}})))))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :animations/swap
                         :handler (fn [db event]
                                    (local next (misa.swap_animation db event.animation event.role))
                                    (local result (reconcile-timer next))
                                    (local fx result.fx)
                                    (when (not= config.persist false)
                                      (tset fx (+ (length fx) 1)
                                            {:data {:active next.animations.active
                                                    :roles next.animations.roles}
                                             :namespace :ui.animation
                                             :type :state/save}))
                                    (tset fx (+ (length fx) 1)
                                          {:event {:type :ui/redraw}
                                           :type :dispatch})
                                    result)})
          (table.insert setup-fx
                        {:type :register/event
                         :name :animations/start
                         :handler (fn [db event]
                                    (start-role db
                                                     (assert event.role
                                                             "animation role is required")))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :animations/stop
                         :handler (fn [db event]
                                    (stop-role db
                                                    (assert event.role
                                                            "animation role is required")))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :animations/tick
                         :handler (fn [db event]
                                    (if (or (not= event.id timer-id)
                                            (not db.animations.timer_running))
                                        nil
                                        (do
                                          ;; Ignore native tick counters: every running role advances exactly once
                                          ;; in the same transaction as the redraw request.
                                          (local ticks {})
                                          (each [role (pairs db.animations.running)]
                                            (when (moving db role)
                                              (tset ticks role
                                                    (+ (or (. db.animations.ticks
                                                              role)
                                                           0)
                                                       1))))
                                          {:patch {:animations {: ticks}}
                                           :fx [{:event {:type :ui/redraw}
                                                 :type :dispatch}]})))})
          {:fx setup-fx})}
