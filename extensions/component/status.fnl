;; Common indicator visual. Semantic label, value, and hotkey classes stay

;; distinct, while low-priority values disappear as the terminal narrows.

(fn span [text style action] {: action : style : text})

{:setup (fn []
          (local setup-fx [])

          (fn render-indicators [model context]
            (local source (or model.indicators {}))
            (local keep {})
            (for [i 1 (length source)] (tset keep i true))

            (fn item-width [item]
              (var width
                   (misa.layout.width (.. (tostring (or item.label "")) " "
                                          (tostring (or item.value "")))))
              (when (and item.hotkey (not= item.hotkey ""))
                (var rendered "")
                (each [_ token (ipairs (or (and misa.keybinding_tokens
                                                (misa.keybinding_tokens item.hotkey))
                                           [{:text item.hotkey}]))]
                  (set rendered (.. rendered token.text)))
                (set width (+ width 1 (misa.layout.width rendered))))
              width)

            (fn total []
              (var (width count) (values 0 0))
              (each [i item (ipairs source)]
                (when (. keep i)
                  (set width (+ width (item-width item)))
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
                (tset spans (+ (length spans) 1)
                      (span (tostring (or item.label "")) :label item.action))
                (tset spans (+ (length spans) 1) (span " " :plain))
                (tset spans (+ (length spans) 1)
                      (span (tostring (or item.value "")) :value item.action))
                (when (and item.hotkey (not= item.hotkey ""))
                  (tset spans (+ (length spans) 1) (span " " :plain))
                  (each [_ key-span (ipairs (or (and misa.render_keybinding
                                                     (misa.render_keybinding item.hotkey))
                                                [(span (tostring item.hotkey)
                                                       :keybinding)]))]
                    (set key-span.action item.action)
                    (tset spans (+ (length spans) 1) key-span)))))
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
