;; Open pure formatting of semantic values, independent of any component role.
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
;; RFC 3339 and Unix instants share one parser, independent of a provider.
(fn timestamp-seconds [value]
  (if (= (type value) :number) (finite value)
      (= (type value) :string)
      (let [(y m d h minute second zone)
            (value:match "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d):(%d%d):(%d%d)(.*)$")]
        (when y
          (local (year month day) (values (tonumber y) (tonumber m) (tonumber d)))
          (local adjusted (- year (if (<= month 2) 1 0)))
          (local era (math.floor (/ adjusted 400)))
          (local yo (- adjusted (* era 400)))
          (local doy (+ (math.floor (/ (+ (* 153 (+ month (if (> month 2) -3 9))) 2) 5)) (- day 1)))
          (local days (- (+ (* era 146097) (* yo 365) (math.floor (/ yo 4))
                            (- (math.floor (/ yo 100))) doy) 719468))
          (local suffix (zone:gsub "^%.%d+" ""))
          (local (sign zh zm) (suffix:match "^([+-])(%d%d):(%d%d)$"))
          (when (and (<= 1 month 12) (<= 1 day 31) (< (tonumber h) 24)
                     (< (tonumber minute) 60) (< (tonumber second) 61)
                     (or (= suffix "Z") (and sign (< (tonumber zh) 24) (< (tonumber zm) 60))))
            (- (+ (* days 86400) (* (tonumber h) 3600) (* (tonumber minute) 60) (tonumber second))
               (if sign (* (if (= sign "+") 1 -1) (+ (* (tonumber zh) 3600) (* (tonumber zm) 60))) 0)))))))

(fn relative-time [instant now]
  (local seconds (- instant now))
  (local minutes (math.ceil (/ (math.abs seconds) 60)))
  (local duration (if (< minutes 1) "<1m"
                     (< minutes 60) (.. minutes "m")
                     (< minutes 1440) (.. (math.floor (/ minutes 60)) "h " (% minutes 60) "m")
                     (.. (math.floor (/ minutes 1440)) "d " (math.floor (/ (% minutes 1440) 60)) "h")))
  (if (= seconds 0) "now" (< seconds 0) (.. duration " ago") (.. "in " duration)))

(fn text-value [text] [(span text)])
(local builtins
       {:text (fn [fact] (assert (= (type fact.value) :string) "text fact requires a string") (text-value fact.value))
        :boolean (fn [fact] (assert (= (type fact.value) :boolean) "boolean fact requires a boolean") (text-value (tostring fact.value)))
        :number (fn [fact] (text-value (if fact.compact (compact fact.value) (tostring (finite fact.value)))))
        :datetime (fn [fact]
                    (local instant (timestamp-seconds fact.value))
                    (local formatted (and instant (misa.local_datetime instant)))
                    (text-value (if formatted
                                    (.. (or fact.prefix "") formatted
                                        (if fact.relative_to (.. " (" (relative-time instant fact.relative_to) ")") ""))
                                    (or fact.fallback "Time unavailable"))))
        :sequence (fn [fact context]
                    (local result [])
                    (each [_ item (ipairs fact.values)]
                      (each [_ part (ipairs (misa.render_value item context))] (table.insert result part)))
                    result)
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
        :timestamp (fn [fact]
                     (local seconds (% (math.floor (/ (finite fact.value) 1000)) 86400))
                     (text-value (string.format "%02d:%02d:%02d" (math.floor (/ seconds 3600))
                                                (% (math.floor (/ seconds 60)) 60) (% seconds 60))))
        :money (fn [fact]
                 (assert (= (type fact.currency) :string) "money requires a currency")
                 (each [_ key (ipairs [:pending :estimated :unknown])]
                   (assert (or (= (. fact key) nil) (= (type (. fact key)) :boolean)) "invalid money flag"))
                 (when fact.pending
                   (assert (= fact.amount nil) "pending money cannot have an amount")
                   (let [pending (text-value :pending)] (lua "return pending")))
                 (local amount (finite fact.amount))
                 (assert (>= amount 0) "money amount must be nonnegative")
                 (local currency (if (= fact.currency :USD) "$" (.. fact.currency " ")))
                 (local value (if (= amount 0) (.. currency "0")
                                  (< amount 0.0001) (.. "<" currency "0.0001")
                                  (.. currency (string.format (if (< amount 1) "%.4f" "%.2f") amount))))
                 (text-value (if (and fact.unknown (= amount 0)) "?"
                                 (.. (if fact.estimated "~" "") value (if fact.unknown " + ?" "")))))})
{:setup (fn []
          (local renderers {})
          (local setup-fx
                 [{:type :register/service :name :timestamp_seconds :value timestamp-seconds}
                  {:type :register/setup-effect :name :register/value-renderer
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

          {:fx setup-fx})}
