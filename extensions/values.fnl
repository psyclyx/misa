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
(fn text-value [text] [(span text)])
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

          {:fx setup-fx})}
