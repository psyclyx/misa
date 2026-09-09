(local definitions (require :misa.definitions))

;; Indicator row composition; general value formatting is owned by values.

(fn span [text style action] {: text : style : action})
(fn text-value [text] [(span text)])
(fn activity [fact context]
  (assert (= (type fact.state) :string) "activity requires a state")
  (let [animation context.activity_animation
        result (text-value fact.state)]
    (when (and (not= fact.state :ready) animation)
      (let [frames animation.frames]
        (assert (and (= (type frames) :table) (> (length frames) 0))
                "animation requires frames")
        (let [phase (or animation.phase 0)]
          (assert (and (= (type phase) :number) (= (% phase 1) 0) (>= phase 0)
                       (< phase (length frames)))
                  "animation phase must name a frame")
          (let [animated (span (if animation.enabled (. frames (+ phase 1))
                                   (or animation.still (. frames 1))))
                distinct (accumulate [found false _ frame (ipairs frames)]
                           (or found (not= frame (. frames 1))))]
            (when (and animation.enabled distinct)
              (assert (<= (length frames) 64)
                      "clock animation supports at most 64 frames")
              (set animated.animation
                   {:id animation.id
                    :interval_ms animation.interval_ms
                    : phase
                    :frames (icollect [_ frame (ipairs frames)] {:text frame})}))
            (table.insert result animated)))))
    result))

(fn item-spans [item context]
  (let [result []]
    (when (not= (or item.representation :label) :value)
      (table.insert result (span (or item.label "") :label item.action))
      (table.insert result (span " " :plain item.action)))
    (each [_ value (ipairs (misa.values.render item.fact context))]
      (let [rendered (misa.snapshot value)]
        (assert (= (type rendered.text) :string)
                "value renderer must return text spans")
        (set rendered.style (or rendered.style :value))
        (set rendered.action (or rendered.action item.action))
        (table.insert result rendered)))
    (when (and item.hotkey (not= item.hotkey ""))
      (table.insert result (span " " :plain item.action))
      (each [_ key-span (ipairs (misa.keybindings.render item.hotkey))]
        (let [next (misa.snapshot key-span)]
          (set next.action item.action)
          (table.insert result next))))
    result))

(fn render-indicators [model context]
  (let [source (or model.indicators {})
        keep {}]
    (for [i 1 (length source)] (tset keep i true))
    (let [rendered (icollect [_ item (ipairs source)]
                     (item-spans item context))
          widths (icollect [_ spans (ipairs rendered)]
                   (misa.layout.width (table.concat (icollect [_ item (ipairs spans)]
                                                      item.text))))]
      (fn total []
        (var (width count) (values 0 0))
        (each [i item (ipairs source)]
          (when (. keep i)
            (set width (+ width (. widths i)))
            (set count (+ count 1))))
        (+ width (* (math.max 0 (- count 1)) 2)))

      (let [columns (math.max 1 (math.floor (or (tonumber context.columns) 80)))]
        (do
          (var finished? false)
          (while (and (not finished?) (> (total) columns))
            (var victim nil)
            (each [i item (ipairs source)]
              (when (and (. keep i)
                         (or (not victim)
                             (< (or (tonumber item.priority) 0)
                                (or (tonumber (. source victim :priority)) 0))
                             (and (= (or (tonumber item.priority) 0)
                                     (or (tonumber (. source victim :priority))
                                         0))
                                  (> i victim))))
                (set victim i)))
            (when (not victim) (set finished? true))
            (when (not finished?) (tset keep victim false))))
        (let [spans {}]
          (each [i item (ipairs source)]
            (when (. keep i)
              (when (> (length spans) 0)
                (tset spans (+ (length spans) 1) (span "  " :plain)))
              (each [_ item-span (ipairs (. rendered i))]
                (tset spans (+ (length spans) 1) item-span))))
          {:lines (or (and (> (length spans) 0) [{: spans}]) {})})))))

(fn build []
  "Declare status-line rendering."
  (let [declarations [{:catalog :value-renderers :id :activity :value activity}]]
    (table.insert declarations
                  {:catalog :components
                   :id :default.status.indicators
                   :value {:render render-indicators}})
    (definitions.build :component.status
      declarations
      {:requirements {:component.status [:values.render]}})))

{:build build}
