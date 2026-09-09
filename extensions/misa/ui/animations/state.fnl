(local timer-id :animation/service)

(fn selected-id [db role]
  "Return the animation identity selected for a role."
  (let [state (assert db.animations "animation state is not initialized")]
    (or (and role (. state.roles role)) state.active)))

(fn moving? [catalog options db role]
  (and options.enabled (> (length (. (assert (. catalog (selected-id db role))
                                             "unknown animation")
                                     :frames)) 1)))

(fn reconcile [catalog options db]
  "Plan the timer transition required by the running animation roles."
  (let [needed (accumulate [needed false role (pairs db.animations.running)
                            &until needed]
                 (moving? catalog options db role))]
    {:patch {:animations (misa.replace (misa.patch db.animations
                                                   {:timer_running needed}))}
     :fx (if (= needed (= db.animations.timer_running true)) [] needed
             [{:completion :animations/tick
               :id timer-id
               :interval_ms options.interval_ms
               :type :timer/start}]
             [{:id timer-id :type :timer/stop}])}))

(fn start [catalog options db role]
  "Start a role at its first frame and reconcile the timer."
  (when (not (. db.animations.running role))
    (reconcile catalog options
               (misa.patch db
                           {:animations {:running {role true} :ticks {role 0}}}))))

(fn stop [catalog options db role]
  "Stop a running role and reconcile the timer."
  (when (. db.animations.running role)
    (reconcile catalog options
               (misa.patch db {:animations {:running {role misa.delete}}}))))

(fn tick [catalog options db event]
  "Advance each moving role once for an active timer event."
  (when (and (= event.id timer-id) db.animations.timer_running)
    {:patch {:animations {:ticks (collect [role (pairs db.animations.running)]
                                   (when (moving? catalog options db role)
                                     (values role
                                             (+ (or (. db.animations.ticks role)
                                                    0)
                                                1))))}}
     :fx [{:event {:type :ui/redraw} :type :dispatch}]}))

{: selected-id : reconcile : start : stop : tick}
