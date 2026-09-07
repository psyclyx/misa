;; Indicator row composition; general value formatting is owned by values.
(fn span [text style action] {: text : style : action})
(fn text-value [text] [(span text)])
(fn activity [fact context]
  (assert (= (type fact.state) :string) "activity requires a state")
  (local animation context.activity_animation)
  (local result (text-value fact.state))
  (when (and (not= fact.state :ready) animation)
    (local frames animation.frames)
    (assert (and (= (type frames) :table) (> (length frames) 0)) "animation requires frames")
    (local phase (or animation.phase 0))
    (assert (and (= (type phase) :number) (= (% phase 1) 0)
                 (>= phase 0) (< phase (length frames))) "animation phase must name a frame")
    (local animated (span (if animation.enabled (. frames (+ phase 1))
                              (or animation.still (. frames 1)))))
    (local distinct (accumulate [found false _ frame (ipairs frames)]
                      (or found (not= frame (. frames 1)))))
    (when (and animation.enabled distinct)
      (assert (<= (length frames) 64) "clock animation supports at most 64 frames")
      (set animated.animation {:id animation.id :interval_ms animation.interval_ms : phase
                               :frames (icollect [_ frame (ipairs frames)] {:text frame})}))
    (table.insert result animated))
  result)
(fn item-spans [item context]
  (local result [])
  (when (not= (or item.representation :label) :value)
    (table.insert result (span (or item.label "") :label item.action))
    (table.insert result (span " " :plain)))
  (each [_ value (ipairs (misa.render_value item.fact context))]
    (local rendered (misa.snapshot value))
    (assert (= (type rendered.text) :string) "value renderer must return text spans")
    (set rendered.style (or rendered.style :value))
    (set rendered.action (or rendered.action item.action))
    (table.insert result rendered))
  (when (and item.hotkey (not= item.hotkey ""))
    (table.insert result (span " " :plain))
    (each [_ key-span (ipairs (or (and misa.render_keybinding (misa.render_keybinding item.hotkey))
                                  [(span (tostring item.hotkey) :keybinding)]))]
      (local next (misa.snapshot key-span))
      (set next.action item.action)
      (table.insert result next)))
  result)
{:setup (fn []
          (assert misa.render_value "component.status requires values")
          (local setup-fx [{:type :register/value-renderer :id :activity :render activity}])

          (fn render-indicators [model context]
            (local source (or model.indicators {}))
            (local keep {})
            (for [i 1 (length source)] (tset keep i true))
            (local rendered (icollect [_ item (ipairs source)] (item-spans item context)))

            (local widths (icollect [_ spans (ipairs rendered)]
                            (misa.layout.width (table.concat (icollect [_ item (ipairs spans)] item.text)))))

            (fn total []
              (var (width count) (values 0 0))
              (each [i item (ipairs source)]
                (when (. keep i)
                  (set width (+ width (. widths i)))
                  (set count (+ count 1))))
              (+ width (* (math.max 0 (- count 1)) 2)))

            (local columns
                   (math.max 1 (math.floor (or (tonumber context.columns) 80))))
            (while (> (total) columns)
              (var victim nil)
              (each [i item (ipairs source)]
                (when (and (. keep i)
                           (or (or (not victim)
                                   (< (or (tonumber item.priority) 0)
                                      (or (tonumber (. source victim :priority))
                                          0)))
                               (and (= (or (tonumber item.priority) 0)
                                       (or (tonumber (. source victim :priority))
                                           0))
                                    (> i victim))))
                  (set victim i)))
              (when (not victim) (lua :break))
              (tset keep victim false))
            (local spans {})
            (each [i item (ipairs source)]
              (when (. keep i)
                (when (> (length spans) 0)
                  (tset spans (+ (length spans) 1) (span "  " :plain)))
                (each [_ item-span (ipairs (. rendered i))]
                  (tset spans (+ (length spans) 1) item-span))))
            {:lines (or (and (> (length spans) 0) [{: spans}]) {})})

          (table.insert setup-fx
                        {:type :register/component
                         :id :default.status.indicators
                         :value {:render render-indicators}})
          {:fx setup-fx})}
