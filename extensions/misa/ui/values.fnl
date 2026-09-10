;; Open pure formatting of semantic values, independent of any component role.
(fn span [text style action] {: text : style : action})
(fn finite [value]
  (assert (and (= (type value) :number) (= value value)
               (< (math.abs value) math.huge))
          "display number must be finite")
  value)

(fn compact [value]
  (finite value)
  (let [unit (accumulate [found nil _ item (ipairs [[1000000000 :G]
                                                    [1000000 :M]
                                                    [1000 :k]])
                          &until found]
               (when (>= (math.abs value) (. item 1)) item))]
    (if unit
        (let [scaled (/ value (. unit 1))]
          (.. (: (string.format (if (>= (math.abs scaled) 10) "%.0f" "%.1f")
                                scaled) :gsub "%.0$" "")
              (. unit 2)))
        (tostring value))))

;; RFC 3339 and Unix instants share one parser, independent of a provider.
(fn timestamp-seconds [value]
  "Parse a Unix or RFC 3339 instant into seconds."
  (if (= (type value) :number) (finite value) (= (type value) :string)
      (let [(y m d h minute second zone) (value:match "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d):(%d%d):(%d%d)(.*)$")]
        (when y
          (let [(year month day) (values (tonumber y) (tonumber m) (tonumber d))
                adjusted (- year (if (<= month 2) 1 0))
                era (math.floor (/ adjusted 400))
                yo (- adjusted (* era 400))
                doy (+ (math.floor (/ (+ (* 153 (+ month (if (> month 2) -3 9)))
                                         2)
                                      5)) (- day 1))
                days (- (+ (* era 146097) (* yo 365) (math.floor (/ yo 4))
                           (- (math.floor (/ yo 100))) doy)
                        719468)
                suffix (zone:gsub "^%.%d+" "")
                (sign zh zm) (suffix:match "^([+-])(%d%d):(%d%d)$")]
            (when (and (<= 1 month 12) (<= 1 day 31) (< (tonumber h) 24)
                       (< (tonumber minute) 60) (< (tonumber second) 61)
                       (or (= suffix :Z)
                           (and sign (< (tonumber zh) 24) (< (tonumber zm) 60))))
              (- (+ (* days 86400) (* (tonumber h) 3600)
                    (* (tonumber minute) 60) (tonumber second))
                 (if sign
                     (* (if (= sign "+") 1 -1)
                        (+ (* (tonumber zh) 3600) (* (tonumber zm) 60)))
                     0))))))))

(fn relative-time [instant now]
  (let [seconds (- instant now)
        minutes (math.ceil (/ (math.abs seconds) 60))
        duration (if (< minutes 1)
                     :<1m
                     (< minutes 60)
                     (.. minutes :m)
                     (< minutes 1440)
                     (.. (math.floor (/ minutes 60)) "h " (% minutes 60) :m)
                     (.. (math.floor (/ minutes 1440)) "d "
                         (math.floor (/ (% minutes 1440) 60)) :h))]
    (if (= seconds 0) :now
        (< seconds 0) (.. duration " ago")
        (.. "in " duration))))

(fn text-value [text] [(span text)])
(fn format-text [fact]
  "Render a text fact as semantic text spans."
  (assert (= (type fact.value) :string) "text fact requires a string")
  (text-value fact.value))

(fn format-boolean [fact]
  "Render a boolean fact as semantic text spans."
  (assert (= (type fact.value) :boolean) "boolean fact requires a boolean")
  (text-value (tostring fact.value)))

(fn format-number [fact]
  "Render a number fact as semantic text spans."
  (text-value (if fact.compact (compact fact.value)
                  (tostring (finite fact.value)))))

(fn format-datetime [fact]
  "Render a datetime fact as semantic text spans."
  (let [instant (timestamp-seconds fact.value)
        formatted (and instant (misa.time.local-datetime instant))]
    (text-value (if formatted
                    (.. (or fact.prefix "") formatted
                        (if fact.relative_to
                            (.. " (" (relative-time instant fact.relative_to)
                                ")")
                            ""))
                    (or fact.fallback "Time unavailable")))))

(fn format-sequence [fact context]
  "Render a sequence fact as semantic text spans."
  (let [result []]
    (each [_ item (ipairs fact.values)]
      (each [_ part (ipairs (misa.values.render item context))]
        (table.insert result part)))
    result))

(fn format-tokens [fact]
  "Render a tokens fact as semantic text spans."
  (text-value (compact fact.value)))

(fn format-duration [fact]
  "Render a duration fact as semantic text spans."
  (let [milliseconds (finite fact.value)]
    (assert (>= milliseconds 0) "duration must be nonnegative")
    (text-value (if (< milliseconds 1000)
                    (.. (math.floor milliseconds) :ms)
                    (< milliseconds 60000)
                    (string.format "%.1fs" (/ milliseconds 1000))
                    (.. (math.floor (/ milliseconds 60000)) "m "
                        (math.floor (/ (% milliseconds 60000) 1000)) :s)))))

(fn format-rate [fact]
  "Render a rate fact as semantic text spans."
  (assert (= (type fact.unit) :string) "rate requires a unit")
  (text-value (.. (string.format "%.1f" (finite fact.value)) " " fact.unit :/s)))

(fn format-ratio [fact]
  "Render a ratio fact as semantic text spans."
  (let [format (if (= fact.unit :tokens) compact (fn [v] (tostring (finite v))))]
    (text-value (.. (if (= fact.used nil) "?" (format fact.used)) "/"
                    (if (= fact.limit nil) "?" (format fact.limit))))))

(fn format-percent [fact]
  "Render a percent fact as semantic text spans."
  (let [value (finite fact.value)]
    (assert (and (>= value 0) (<= value 100))
            "percentage must be within 0..100")
    (text-value (.. (string.format "%.0f%%" value)
                    (if (= fact.basis :remaining) " left" "")))))

(fn format-unavailable []
  "Render a unavailable fact as semantic text spans."
  (text-value :unavailable))

(fn format-timestamp [fact]
  "Render a timestamp fact as semantic text spans."
  (let [seconds (% (math.floor (/ (finite fact.value) 1000)) 86400)]
    (text-value (string.format "%02d:%02d:%02d" (math.floor (/ seconds 3600))
                               (% (math.floor (/ seconds 60)) 60) (% seconds 60)))))

(fn format-money [fact]
  "Render a money fact as semantic text spans."
  (assert (= (type fact.currency) :string) "money requires a currency")
  (each [_ key (ipairs [:pending :estimated :unknown])]
    (assert (or (= (. fact key) nil) (= (type (. fact key)) :boolean))
            "invalid money flag"))
  (if fact.pending
      (do
        (assert (= fact.amount nil) "pending money cannot have an amount")
        (text-value :pending))
      (let [amount (finite fact.amount)]
        (assert (>= amount 0) "money amount must be nonnegative")
        (let [currency (if (= fact.currency :USD) "$" (.. fact.currency " "))
              value (if (= amount 0) (.. currency :0) (< amount 0.0001)
                        (.. "<" currency :0.0001)
                        (.. currency
                            (string.format (if (< amount 1) "%.4f" "%.2f")
                                           amount)))]
          (text-value (if (and fact.unknown (= amount 0)) "?"
                          (.. (if fact.estimated "~" "") value
                              (if fact.unknown " + ?" ""))))))))

(local builtins {:text format-text
                 :boolean format-boolean
                 :number format-number
                 :datetime format-datetime
                 :sequence format-sequence
                 :tokens format-tokens
                 :duration format-duration
                 :rate format-rate
                 :ratio format-ratio
                 :percent format-percent
                 :unavailable format-unavailable
                 :timestamp format-timestamp
                 :money format-money})

(fn values-render [fact context]
  "Render a typed fact using its configured implementation."
  (assert (and (= (type fact) :table) (= (type fact.type) :string))
          "value renderer requires a typed fact")
  ((assert (. (misa.catalog :value-renderers) fact.type)
           (.. "unknown value renderer for: " fact.type)) fact (or context {})))

(fn validate-renderer [_ render]
  "Require a callable typed-value renderer."
  (assert (= (type render) :function) "value renderer must be a function"))

{: builtins : timestamp-seconds : validate-renderer : values-render}
