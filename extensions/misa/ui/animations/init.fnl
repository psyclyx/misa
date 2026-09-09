(local animation-state (require :misa.ui.animations.state))
(local definitions (require :misa.definitions))

;; Animation registry with clock-driven spans and optional explicit timer events.
;; Registrations are immutable once
;; app/start begins; selections, role ticks, and running timers are transactional.

(fn animations-presentation-value [enabled interval-ms inputs query]
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
  (assert (and (= (type animation) :table) (= (type animation.frames) :table)
               (> (length animation.frames) 0))
          "animation requires frames")
  (each [_ frame (ipairs animation.frames)]
    (assert (= (type frame) :string) "animation frames must be text"))
  (assert (or (= animation.still nil) (= (type animation.still) :string))
          "invalid still frame"))

(fn build [context]
  "Build the declarations for animations."
  (let [declarations []
        config (let [value (. (or context.config {}) :animations)]
                 (if (= (type value) :table) value {}))
        configured (or (and (= (type config.default) :string) config.default)
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
      (table.insert declarations
                    (let [definition {:id :animations/presentation
                                      :inputs (fn [query]
                                                [[:db/path :animations :active]
                                                 [:db/path
                                                  :animations
                                                  :roles
                                                  (. query 2)]])
                                      :compute (fn [inputs query]
                                                 (animations-presentation-value enabled
                                                                                interval-ms
                                                                                inputs
                                                                                query))}]
                      {:catalog :subscriptions
                       :id (. definition :id)
                       :value definition}))
      (table.insert declarations
                    {:catalog :services
                     :id :animations.state
                     :value animations-state})
      (table.insert declarations
                    {:catalog :services
                     :id :animations.lookup
                     :value (fn [db role]
                              (animations-lookup db role))})
      (table.insert declarations
                    {:catalog :services
                     :id :animations.frame
                     :value (fn [db role tick]
                              (animations-frame enabled db role tick))})
      (table.insert declarations
                    {:catalog :services
                     :id :animations.span
                     :value (fn [db role options]
                              (animations-span enabled interval-ms db role
                                               options))})
      (table.insert declarations
                    {:catalog :services
                     :id :animations.swap
                     :value animations-swap})
      (let [options {:enabled enabled :interval_ms interval-ms}]
        (table.insert declarations
                      {:catalog :events
                       :value {:event :app/start
                               :handler (fn [db]
                                          (app-start config configured
                                                     configured-roles db))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :animations/loaded
                               :handler (fn [db event]
                                          (animations-loaded options db event))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :animations/swap
                               :handler (fn [db event]
                                          (animations-swap-handler config
                                                                   options db
                                                                   event))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :animations/start
                               :handler (fn [db event]
                                          (animation-state.start (misa.catalog :animations)
                                                                 options db
                                                                 (assert event.role
                                                                         "animation role is required")))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :animations/stop
                               :handler (fn [db event]
                                          (animation-state.stop (misa.catalog :animations)
                                                                options db
                                                                (assert event.role
                                                                        "animation role is required")))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :animations/tick
                               :handler (fn [db event]
                                          (animation-state.tick (misa.catalog :animations)
                                                                options db event))}})
        (definitions.build :animations
          declarations
          {:validators {:animations validate-animation}})))))

{:build build}
