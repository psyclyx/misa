;; Animation registry and timer-backed service. Registrations are immutable once

;; app/start begins; selections, role ticks, and running timers are transactional.

{:setup (fn [context]
          (var (entries registrations-sealed) (values {} false))
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

          (fn misa.reg_animation [id animation]
            (assert (not registrations-sealed)
                    "animation registrations are sealed")
            (assert (and (and (= (type id) :string) (not= id ""))
                         (= (type animation) :table))
                    "invalid animation")
            (assert (and (= (type animation.frames) :table)
                         (> (length animation.frames) 0))
                    "animation frames must be nonempty")
            (each [_ frame (ipairs animation.frames)]
              (assert (= (type frame) :string)
                      "animation frames must be strings"))
            (assert (or (= animation.still nil)
                        (= (type animation.still) :string))
                    "animation still frame must be a string")
            (assert (= (. entries id) nil) (.. "duplicate animation: " id))
            (tset entries id animation)
            nil)

          (fn animation-id [db role]
            (local state
                   (assert db.animations "animation state is not initialized"))
            (or (and role (. state.roles role)) state.active))

          (fn misa.animation [db role]
            (assert (= (type db) :table)
                    "animation resolution requires initialized db")
            (local id (animation-id db role))
            (assert (. entries id) (.. "unknown animation: " (tostring id))))

          (fn misa.animation_frame [db role tick]
            (local animation (misa.animation db role))
            (local frames animation.frames)
            (if (not enabled) (or animation.still (. frames 1))
                (do
                  (var current tick)
                  (when (= current nil)
                    (set current (or (. (or db.animations.ticks {})
                                        (or role :default))
                                     0)))
                  (. frames (+ (% current (length frames)) 1)))))

          (fn misa.swap_animation [db id role]
            (assert (. entries id) (.. "unknown animation: " (tostring id)))
            (if role (tset db.animations.roles role id)
                (set db.animations.active id))
            nil)

          (local timer-id :animation/service)

          (fn moving [db role]
            (and enabled (> (length (. (misa.animation db role) :frames)) 1)))

          (fn reconcile-timer [db]
            (var needed false)
            (each [role (pairs db.animations.running)]
              (when (moving db role) (set needed true) (lua :break)))
            (if (= needed (= db.animations.timer_running true)) {}
                (do
                  (set db.animations.timer_running needed)
                  (or (and needed
                           [{:completion :animations/tick
                             :id timer-id
                             :interval_ms interval-ms
                             :type :timer/start}])
                      [{:id timer-id :type :timer/stop}]))))

          (fn start-role [db role]
            (local running db.animations.running)
            (if (. running role) {}
                (do
                  (tset running role true)
                  (tset db.animations.ticks role 0)
                  (reconcile-timer db))))

          (fn stop-role [db role]
            (local running db.animations.running)
            (if (not (. running role)) {}
                (do
                  (tset running role nil)
                  (reconcile-timer db))))

          (misa.reg_interceptor {:before (fn [tx]
                                           (when (= tx.event.type :app/start)
                                             (set registrations-sealed true)
                                             (when (not tx.db.animations)
                                               (local roles {})
                                               (each [role id (pairs configured-roles)]
                                                 (assert (and (and (and (= (type role)
                                                                           :string)
                                                                        (not= role
                                                                              ""))
                                                                   (= (type id)
                                                                      :string))
                                                              (. entries id))
                                                         "invalid configured animation role")
                                                 (tset roles role id))
                                               (set tx.db.animations
                                                    {:active configured
                                                     : roles
                                                     :running {}
                                                     :ticks {}})))
                                           tx)
                                 :id :animations/initialize})
          (misa.reg_event :app/start
                          (fn [db]
                            (assert (. entries db.animations.active)
                                    (.. "unknown configured animation: "
                                        (tostring db.animations.active)))
                            (if (= config.persist false) {: db}
                                {: db
                                 :fx [{:completion :animations/loaded
                                       :namespace :ui.animation
                                       :type :state/load}]})))
          (misa.reg_event :animations/loaded
                          (fn [db event]
                            (if (or (= event.found false)
                                    (= event.data misa.json_null))
                                {: db}
                                (do
                                  (assert (and (= (type event.data) :table)
                                               (= (type event.data.active)
                                                  :string))
                                          "invalid persisted animation")
                                  (when (. entries event.data.active)
                                    (set db.animations.active event.data.active))
                                  (each [role id (pairs (or (and (= (type event.data.roles)
                                                                    :table)
                                                                 event.data.roles)
                                                            {}))]
                                    (when (and (= (type role) :string)
                                               (. entries id))
                                      (tset db.animations.roles role id)))
                                  {: db :fx (reconcile-timer db)}))))
          (misa.reg_event :animations/swap
                          (fn [db event]
                            (misa.swap_animation db event.animation event.role)
                            (local fx (reconcile-timer db))
                            (when (not= config.persist false)
                              (tset fx (+ (length fx) 1)
                                    {:data {:active db.animations.active
                                            :roles db.animations.roles}
                                     :namespace :ui.animation
                                     :type :state/save}))
                            (tset fx (+ (length fx) 1)
                                  {:event {:type :ui/redraw} :type :dispatch})
                            {: db : fx}))
          (misa.reg_event :animations/start
                          (fn [db event]
                            {: db
                             :fx (start-role db
                                             (assert event.role
                                                     "animation role is required"))}))
          (misa.reg_event :animations/stop
                          (fn [db event]
                            {: db
                             :fx (stop-role db
                                            (assert event.role
                                                    "animation role is required"))}))
          (misa.reg_event :animations/tick
                          (fn [db event]
                            (if (or (not= event.id timer-id)
                                    (not db.animations.timer_running))
                                nil
                                (do
                                  ;; Ignore native tick counters: every running role advances exactly once
                                  ;; in the same transaction as the redraw request.
                                  (each [role (pairs db.animations.running)]
                                    (when (moving db role)
                                      (tset db.animations.ticks role
                                            (+ (or (. db.animations.ticks role)
                                                   0)
                                               1))))
                                  {: db
                                   :fx [{:event {:type :ui/redraw}
                                         :type :dispatch}]}))))
          (misa.reg_event :agent/status
                          (fn [db event]
                            (local fx
                                   (or (and (= event.status :ready)
                                            (stop-role db :status))
                                       (start-role db :status)))
                            {: db : fx}))
          nil)}

