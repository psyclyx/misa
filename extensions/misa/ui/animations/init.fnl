(local definitions (require :misa.definitions))

;; Animation registry with clock-driven spans and optional explicit timer events.
;; Registrations are immutable once
;; app/start begins; selections, role ticks, and running timers are transactional.

(fn build [context]
  "Build the declarations for animations."
  (let [declarations []]
    (var config (or (and (= (type context.config) :table)
                         context.config.animations) nil))
    (set config (or (and (= (type config) :table) config) {}))
    (let [configured (or (and (= (type config.default) :string) config.default)
                         :default)
          configured-roles (or (and (= (type config.roles) :table) config.roles)
                               {})
          interval-ms (or config.interval_ms 160)]
      (assert (or (= config.enabled nil) (= (type config.enabled) :boolean))
              "animations.enabled must be a boolean")
      (let [enabled (not= config.enabled false)]
        (assert (and (= (type interval-ms) :number) (>= interval-ms 10)
                     (<= interval-ms 60000) (= (% interval-ms 1) 0))
                "animations.interval_ms must be an integer from 10 through 60000")

        (fn animation-id [db role]
          (let [state (assert db.animations
                              "animation state is not initialized")]
            (or (and role (. state.roles role)) state.active)))

        (table.insert declarations
                      (let [definition {:id :animations/presentation
                                        :inputs (fn [query]
                                                  [[:db/path
                                                    :animations
                                                    :active]
                                                   [:db/path
                                                    :animations
                                                    :roles
                                                    (. query 2)]])
                                        :compute (fn [inputs query]
                                                   (let [id (or (. inputs 2)
                                                                (. inputs 1))]
                                                     (when id
                                                       (let [animation (assert (. (misa.catalog :animations)
                                                                                  id)
                                                                               "unknown animation")]
                                                         {: enabled
                                                          :frames animation.frames
                                                          :still animation.still
                                                          :interval_ms interval-ms
                                                          :phase 0
                                                          :id (.. :animation/
                                                                  (. query 2))}))))}]
                        {:catalog :subscriptions
                         :id (. definition :id)
                         :value definition}))
        (table.insert declarations
                      {:catalog :services
                       :id :animations.state
                       :value (fn [db role]
                                "Return the selected animation and its current clock state."
                                (misa.sub db
                                          [:animations/presentation
                                           (or role :default)]))})
        (table.insert declarations
                      {:catalog :services
                       :id :animations.lookup
                       :value (fn [db role]
                                "Resolve the animation implementation for a role."
                                (assert (= (type db) :table)
                                        "animation resolution requires initialized db")
                                (let [id (animation-id db role)]
                                  (assert (. (misa.catalog :animations) id)
                                          (.. "unknown animation: "
                                              (tostring id)))))})
        (table.insert declarations
                      {:catalog :services
                       :id :animations.frame
                       :value (fn [db role tick]
                                "Return the frame at a role's tick, respecting disabled animation."
                                (let [animation (misa.animations.lookup db role)
                                      frames animation.frames]
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
                                           (+ (% current (length frames)) 1))))))})
        (table.insert declarations
                      {:catalog :services
                       :id :animations.span
                       :value (fn [db role options]
                                "Build a span whose animation is driven by the presentation clock."
                                (let [animation (misa.animations.lookup db role)
                                      frames animation.frames
                                      opts (or options {})
                                      phase (or opts.phase 0)]
                                  (assert (and (= (type phase) :number)
                                               (= (% phase 1) 0) (>= phase 0)
                                               (< phase (length frames)))
                                          "animation phase must name a frame")
                                  (let [span {:text (or (and (not enabled)
                                                             (or animation.still
                                                                 (. frames 1)))
                                                        (. frames (+ phase 1)))
                                              :style opts.style}]
                                    (var distinct false)
                                    (each [_ frame (ipairs frames)]
                                      (when (not= frame (. frames 1))
                                        (set distinct true)))
                                    (when (and enabled distinct)
                                      (let [id (or opts.id
                                                   (.. :animation/
                                                       (or role :default)))]
                                        (assert (and (= (type id) :string)
                                                     (> (length id) 0)
                                                     (<= (length id) 256))
                                                "animation ID must contain 1 through 256 bytes")
                                        (assert (<= (length frames) 64)
                                                "clock animation supports at most 64 frames")
                                        (let [projected {}]
                                          (each [_ frame (ipairs frames)]
                                            (table.insert projected
                                                          {:text frame}))
                                          (set span.animation
                                               {: id
                                                :interval_ms interval-ms
                                                : phase
                                                :frames projected}))))
                                    span)))})
        (table.insert declarations
                      {:catalog :services
                       :id :animations.swap
                       :value (fn [db id role]
                                "Return a database patch selecting an animation for a role."
                                (assert (. (misa.catalog :animations) id)
                                        (.. "unknown animation: " (tostring id)))
                                (misa.patch db
                                            {:animations (if role
                                                             {:roles {role id}}
                                                             {:active id})}))})
        (let [timer-id :animation/service]
          (fn moving [db role]
            (and enabled (> (length (. (misa.animations.lookup db role) :frames))
                            1)))

          (fn reconcile-timer [db]
            (var needed false)
            (each [role (pairs db.animations.running)]
              (when (moving db role) (set needed true) (lua :break)))
            {:patch {:animations (misa.replace (misa.patch db.animations
                                                           {:timer_running needed}))}
             :fx (if (= needed (= db.animations.timer_running true)) {}
                     (or (and needed
                              [{:completion :animations/tick
                                :id timer-id
                                :interval_ms interval-ms
                                :type :timer/start}])
                         [{:id timer-id :type :timer/stop}]))})

          (fn start-role [db role]
            (let [running db.animations.running]
              (when (not (. running role))
                (reconcile-timer (misa.patch db
                                             {:animations {:running {role true}
                                                           :ticks {role 0}}})))))

          (fn stop-role [db role]
            (let [running db.animations.running]
              (when (. running role)
                (reconcile-timer (misa.patch db
                                             {:animations {:running {role misa.delete}}})))))

          (table.insert declarations
                        {:catalog :events
                         :value {:event :app/start
                                 :handler (fn [db]
                                            (let [initial (when (not db.animations)
                                                            {:active configured
                                                             :running {}
                                                             :ticks {}
                                                             :roles (collect [role id (pairs configured-roles)]
                                                                      (do
                                                                        (assert (and (= (type role)
                                                                                        :string)
                                                                                     (not= role
                                                                                           "")
                                                                                     (= (type id)
                                                                                        :string)
                                                                                     (. (misa.catalog :animations)
                                                                                        id))
                                                                                "invalid configured animation role")
                                                                        (values role
                                                                                id)))})
                                                  active (. (or db.animations
                                                                initial)
                                                            :active)]
                                              (assert (. (misa.catalog :animations)
                                                         active)
                                                      (.. "unknown configured animation: "
                                                          (tostring active)))
                                              {:patch (when initial
                                                        {:animations (misa.replace initial)})
                                               :fx (when (not= config.persist
                                                               false)
                                                     [{:completion :animations/loaded
                                                       :namespace :ui.animation
                                                       :type :state/load}])}))}})
          (table.insert declarations
                        {:catalog :events
                         :value {:event :animations/loaded
                                 :handler (fn [db event]
                                            (if (or (= event.found false)
                                                    (= event.data
                                                       misa.json-null))
                                                nil
                                                (do
                                                  (assert (and (= (type event.data)
                                                                  :table)
                                                               (= (type event.data.active)
                                                                  :string))
                                                          "invalid persisted animation")
                                                  (let [roles {}]
                                                    (each [role id (pairs (or (and (= (type event.data.roles)
                                                                                      :table)
                                                                                   event.data.roles)
                                                                              {}))]
                                                      (when (and (= (type role)
                                                                    :string)
                                                                 (. (misa.catalog :animations)
                                                                    id))
                                                        (tset roles role id)))
                                                    (reconcile-timer (misa.patch db
                                                                                 {:animations {: roles
                                                                                               :active (when (. (misa.catalog :animations)
                                                                                                                event.data.active)
                                                                                                         event.data.active)}}))))))}})
          (table.insert declarations
                        {:catalog :events
                         :value {:event :animations/swap
                                 :handler (fn [db event]
                                            (let [next (misa.animations.swap db
                                                                             event.animation
                                                                             event.role)
                                                  result (reconcile-timer next)
                                                  fx result.fx]
                                              (when (not= config.persist false)
                                                (tset fx (+ (length fx) 1)
                                                      {:data {:active next.animations.active
                                                              :roles next.animations.roles}
                                                       :namespace :ui.animation
                                                       :type :state/save}))
                                              (tset fx (+ (length fx) 1)
                                                    {:event {:type :ui/redraw}
                                                     :type :dispatch})
                                              result))}})
          (table.insert declarations
                        {:catalog :events
                         :value {:event :animations/start
                                 :handler (fn [db event]
                                            (start-role db
                                                        (assert event.role
                                                                "animation role is required")))}})
          (table.insert declarations
                        {:catalog :events
                         :value {:event :animations/stop
                                 :handler (fn [db event]
                                            (stop-role db
                                                       (assert event.role
                                                               "animation role is required")))}})
          (table.insert declarations
                        {:catalog :events
                         :value {:event :animations/tick
                                 :handler (fn [db event]
                                            (if (or (not= event.id timer-id)
                                                    (not db.animations.timer_running))
                                                nil
                                                (do
                                                  ;; Ignore native tick counters: every running role advances exactly once
                                                  ;; in the same transaction as the redraw request.
                                                  (let [ticks {}]
                                                    (each [role (pairs db.animations.running)]
                                                      (when (moving db role)
                                                        (tset ticks role
                                                              (+ (or (. db.animations.ticks
                                                                        role)
                                                                     0)
                                                                 1))))
                                                    {:patch {:animations {: ticks}}
                                                     :fx [{:event {:type :ui/redraw}
                                                           :type :dispatch}]}))))}})
          (definitions.build :animations
            declarations
            {:validators {:animations (fn [_ animation]
                                        (assert (and (= (type animation) :table)
                                                     (= (type animation.frames)
                                                        :table)
                                                     (> (length animation.frames)
                                                        0))
                                                "animation requires frames")
                                        (each [_ frame (ipairs animation.frames)]
                                          (assert (= (type frame) :string)
                                                  "animation frames must be text"))
                                        (assert (or (= animation.still nil)
                                                    (= (type animation.still)
                                                       :string))
                                                "invalid still frame"))}}))))))

{: build}
