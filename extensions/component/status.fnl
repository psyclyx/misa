;; Common indicator visual. Semantic label, value, and hotkey classes stay

;; distinct, while low-priority values disappear as the terminal narrows.

(fn span [text style action] {: action : style : text})

(fn value-text [value]
  (if (= (type value) :table)
      (if value.spans
          (let [pieces {}]
            (each [_ item (ipairs value.spans)]
              (table.insert pieces (value-text item)))
            (table.concat pieces))
          (do
            (assert (= (type value.text) :string)
                    "indicator span requires text")
            value.text))
      (tostring (or value ""))))

(fn append-value [spans value style action]
  (if (= (type value) :table)
      (if value.spans
          (each [_ item (ipairs value.spans)]
            (append-value spans item style action))
          (let [result (misa.snapshot value)]
            (value-text result)
            (set result.style (or result.style style))
            (set result.action (or result.action action))
            (table.insert spans result)))
      (table.insert spans (span (value-text value) style action))))

(fn item-spans [item]
  ;; Indicator data is semantic; this component owns the default presentation.
  (local result {})
  (local representation (or item.representation :label))
  (when (not= representation :value)
    (tset result (+ (length result) 1)
          (span (tostring (or item.label "")) :label item.action))
    (tset result (+ (length result) 1) (span " " :plain)))
  (append-value result item.value :value item.action)
  (when (and item.hotkey (not= item.hotkey ""))
    (tset result (+ (length result) 1) (span " " :plain))
    (each [_ key-span (ipairs (or (and misa.render_keybinding
                                       (misa.render_keybinding item.hotkey))
                                  [(span (tostring item.hotkey)
                                         :keybinding)]))]
      (local next (misa.snapshot key-span))
      (set next.action item.action)
      (tset result (+ (length result) 1) next)))
  result)

{:setup (fn []
          (local setup-fx [])

          (fn render-indicators [model context]
            (local source (or model.indicators {}))
            (local keep {})
            (for [i 1 (length source)] (tset keep i true))
            (local rendered (icollect [_ item (ipairs source)] (item-spans item)))

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
          ;; Compatibility for custom profiles using the former metrics role.
          (table.insert setup-fx
                        {:type :register/component
                         :id :default.status.metrics
                         :value {:render (fn [model]
                                           (local spans {})
                                           (each [index metric (ipairs (or model.metrics
                                                                           {}))]
                                             (when (> index 1)
                                               (tset spans (+ (length spans) 1)
                                                     (span "  " :plain)))
                                             (tset spans (+ (length spans) 1)
                                                   (span (tostring (or metric.prefix
                                                                       ""))
                                                         :dim))
                                             (tset spans (+ (length spans) 1)
                                                   (span " " :plain))
                                             (tset spans (+ (length spans) 1)
                                                   (span (tostring (or metric.value
                                                                       ""))
                                                         (or metric.style
                                                             :plain))))
                                           {:lines (or (and (> (length spans) 0)
                                                            [{: spans}])
                                                       {})})}})
          {:fx setup-fx})}
