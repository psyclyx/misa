(local animation-state (require :misa.ui.animations.state))

;; Animation registry with clock-driven spans and optional explicit timer events.
;; Registrations are immutable once
;; app/start begins; selections, role ticks, and running timers are transactional.

(fn animations-presentation-value [enabled interval-ms inputs query]
  "Describe an animation selection and its presentation clock."
  (let [id (or (. inputs 2) (. inputs 1))]
    (when id
      (let [animation (assert (. (misa.catalog :animations) id)
                              "unknown animation")]
        {: enabled
         :frames animation.frames
         :still animation.still
         :interval_ms interval-ms
         :phase 0
         :id (.. :animation/ (. query 2))}))))

(fn animations-state [db role]
  "Return the selected animation and its current clock state."
  (misa.sub db [:animations/presentation (or role :default)]))

(fn animations-lookup [db role]
  "Resolve the animation implementation for a role."
  (assert (= (type db) :table) "animation resolution requires initialized db")
  (let [id (animation-state.selected-id db role)]
    (assert (. (misa.catalog :animations) id)
            (.. "unknown animation: " (tostring id)))))

(fn animations-frame [enabled db role tick]
  "Return the frame at a role's tick, respecting disabled animation."
  (let [animation (misa.animations.lookup db role)
        frames animation.frames]
    (if (not enabled)
        (or animation.still (. frames 1))
        (do
          (var current tick)
          (when (= current nil)
            (set current (or (. (or db.animations.ticks {}) (or role :default))
                             0)))
          (. frames (+ (% current (length frames)) 1))))))

(fn animations-span [enabled interval-ms db role options]
  "Build a span whose animation is driven by the presentation clock."
  (let [animation (misa.animations.lookup db role)
        frames animation.frames
        opts (or options {})
        phase (or opts.phase 0)]
    (assert (and (= (type phase) :number) (= (% phase 1) 0) (>= phase 0)
                 (< phase (length frames)))
            "animation phase must name a frame")
    (let [span {:text (or (and (not enabled) (or animation.still (. frames 1)))
                          (. frames (+ phase 1)))
                :style opts.style}]
      (var distinct false)
      (each [_ frame (ipairs frames)]
        (when (not= frame (. frames 1))
          (set distinct true)))
      (when (and enabled distinct)
        (let [id (or opts.id (.. :animation/ (or role :default)))]
          (assert (and (= (type id) :string) (> (length id) 0)
                       (<= (length id) 256))
                  "animation ID must contain 1 through 256 bytes")
          (assert (<= (length frames) 64)
                  "clock animation supports at most 64 frames")
          (let [projected {}]
            (each [_ frame (ipairs frames)]
              (table.insert projected {:text frame}))
            (set span.animation
                 {: id :interval_ms interval-ms : phase :frames projected}))))
      span)))

(fn animations-swap [db id role]
  "Return a database patch selecting an animation for a role."
  (assert (. (misa.catalog :animations) id)
          (.. "unknown animation: " (tostring id)))
  (misa.patch db {:animations (if role
                                  {:roles {role id}}
                                  {:active id})}))

(fn app-start [config configured configured-roles db]
  "Initialize selected presentation state and request persisted choices."
  (let [initial (when (not db.animations)
                  {:active configured
                   :running {}
                   :ticks {}
                   :roles (collect [role id (pairs configured-roles)]
                            (do
                              (assert (and (= (type role) :string)
                                           (not= role "") (= (type id) :string)
                                           (. (misa.catalog :animations) id))
                                      "invalid configured animation role")
                              (values role id)))})
        active (. (or db.animations initial) :active)]
    (assert (. (misa.catalog :animations) active)
            (.. "unknown configured animation: " (tostring active)))
    {:patch (when initial
              {:animations (misa.replace initial)})
     :fx (when (not= config.persist false)
           [{:completion :animations/loaded
             :namespace :ui.animation
             :type :state/load}])}))

(fn animations-loaded [options db event]
  "Restore available animation choices and reconcile timers."
  (if (or (= event.found false) (= event.data misa.json-null))
      nil
      (do
        (assert (and (= (type event.data) :table)
                     (= (type event.data.active) :string))
                "invalid persisted animation")
        (let [roles {}]
          (each [role id (pairs (or (and (= (type event.data.roles) :table)
                                         event.data.roles)
                                    {}))]
            (when (and (= (type role) :string)
                       (. (misa.catalog :animations) id))
              (tset roles role id)))
          (animation-state.reconcile (misa.catalog :animations) options
                                     (misa.patch db
                                                 {:animations {: roles
                                                               :active (when (. (misa.catalog :animations)
                                                                                event.data.active)
                                                                         event.data.active)}}))))))

(fn animations-swap-handler [config options db event]
  "Select an animation and plan persistence and timer effects."
  (let [next (misa.animations.swap db event.animation event.role)
        result (animation-state.reconcile (misa.catalog :animations) options
                                          next)
        fx result.fx]
    (when (not= config.persist false)
      (tset fx (+ (length fx) 1)
            {:data {:active next.animations.active
                    :roles next.animations.roles}
             :namespace :ui.animation
             :type :state/save}))
    (tset fx (+ (length fx) 1) {:event {:type :ui/redraw} :type :dispatch})
    result))

(fn validate-animation [_ animation]
  "Require textual animation frames and a valid still frame."
  (assert (and (= (type animation) :table) (= (type animation.frames) :table)
               (> (length animation.frames) 0))
          "animation requires frames")
  (each [_ frame (ipairs animation.frames)]
    (assert (= (type frame) :string) "animation frames must be text"))
  (assert (or (= animation.still nil) (= (type animation.still) :string))
          "invalid still frame"))

{: animations-frame
 : animations-loaded
 : animations-lookup
 : animations-presentation-value
 : animations-span
 : animations-state
 : animations-swap
 : animations-swap-handler
 : app-start
 : validate-animation}
