;; Typed facts are formatted here; domain queries never construct display strings.
(fn span [text style action] {: text : style : action})
(fn finite [value]
  (assert (and (= (type value) :number) (= value value) (< (math.abs value) math.huge))
          "display number must be finite")
  value)
(fn compact [value]
  (finite value)
  (local unit (accumulate [found nil _ item (ipairs [[1000000000 :G] [1000000 :M] [1000 :k]]) &until found]
                (when (>= (math.abs value) (. item 1)) item)))
  (if unit
      (let [scaled (/ value (. unit 1))]
        (.. (: (string.format (if (>= (math.abs scaled) 10) "%.0f" "%.1f") scaled) :gsub "%.0$" "") (. unit 2)))
      (tostring value)))
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
(local builtins
       {:text (fn [fact] (assert (= (type fact.value) :string) "text fact requires a string") (text-value fact.value))
        :boolean (fn [fact] (assert (= (type fact.value) :boolean) "boolean fact requires a boolean") (text-value (tostring fact.value)))
        :number (fn [fact] (text-value (tostring (finite fact.value))))
        :tokens (fn [fact] (text-value (compact fact.value)))
        :ratio (fn [fact]
                 (local format (if (= fact.unit :tokens) compact (fn [v] (tostring (finite v)))))
                 (text-value (.. (if (= fact.used nil) "?" (format fact.used)) "/"
                                 (if (= fact.limit nil) "?" (format fact.limit)))))
        :percent (fn [fact]
                   (local value (finite fact.value))
                   (assert (and (>= value 0) (<= value 100)) "percentage must be within 0..100")
                   (text-value (.. (string.format "%.0f%%" value) (if (= fact.basis :remaining) " left" ""))))
        :unavailable (fn [] (text-value "unavailable"))
        :activity activity
        :money (fn [fact]
                 (local amount (finite fact.amount))
                 (assert (>= amount 0) "money amount must be nonnegative")
                 (assert (= (type fact.currency) :string) "money requires a currency")
                 (local currency (if (= fact.currency :USD) "$" (.. fact.currency " ")))
                 (local value (if (= amount 0) (.. currency "0")
                                  (< amount 0.0001) (.. "<" currency "0.0001")
                                  (.. currency (string.format (if (< amount 1) "%.4f" "%.2f") amount))))
                 (text-value (if (and fact.unknown (= amount 0)) "?"
                                 (.. (if fact.estimated "~" "") value (if fact.unknown " + ?" "")))))})
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
          (local renderers {})
          (local setup-fx
                 [{:type :register/setup-effect :name :register/value-renderer
                   :handler (fn [effect]
                              (assert (and (= (type effect.id) :string) (not= effect.id "")
                                           (= (type effect.render) :function)) "value renderer requires id and render")
                              (assert (= (. renderers effect.id) nil) "duplicate value renderer")
                              (tset renderers effect.id effect.render) nil)}
                  {:type :register/service :name :render_value
                   :value (fn [fact context]
                            (assert (and (= (type fact) :table) (= (type fact.type) :string))
                                    "value renderer requires a typed fact")
                            ((assert (. renderers fact.type) (.. "unknown value type: " fact.type))
                             fact (or context {})))}])
          (each [id render (pairs builtins)]
            (table.insert setup-fx {:type :register/value-renderer : id : render}))

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
