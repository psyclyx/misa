;; Reasoning-effort affordances over the generic request-options policy.

(local option-name :reasoning_effort)

(fn choices [db]
  "Return reasoning effort choices for the selected model."
  (or (and misa.request-options misa.request-options.choices
           (misa.request-options.choices db option-name)) {}))

(fn unavailable [db]
  "Report that the selected model does not support reasoning effort."
  (let [model (or (and db.models db.models.selected) "selected model")]
    {:fx [{:event {:level :info
                   :problem {:code :unsupported
                             :kind :request_option
                             : model
                             :option option-name}
                   :text (.. "reasoning effort is not supported by " model)
                   :type :transcript/harness}
           :type :dispatch}
          {:type :terminal/read}]}))

(fn selected-effort-select [db]
  "Return the currently selected reasoning effort."
  (misa.request-options.value db option-name))

(fn complete-effort-select [_ db]
  "Return completion items for reasoning effort."
  (let [result {}]
    (each [_ value (ipairs (choices db))]
      (tset result (+ (length result) 1)
            {:label (tostring value) :value (tostring value)}))
    result))

(fn on-effort-select [db event]
  "Validate and select the requested reasoning effort."
  (let [available (choices db)]
    (if (= (length available) 0)
        (unavailable db)
        (let [requested (or (and (= (type event.arguments) :string)
                                 (event.arguments:match "^%s*(%S+)%s*$"))
                            nil)
              found (accumulate [selected nil _ value (ipairs available)
                                 &until selected]
                      (when (= (tostring value) requested)
                        {: value}))]
          (assert found "unsupported reasoning effort for the selected model")
          {:fx [{:type :dispatch
                 :event {:type :request-options/select
                         :name option-name
                         :value found.value}}
                {:type :terminal/read}]}))))

(fn on-effort-cycle [db]
  "Select the next supported reasoning effort."
  (let [available (choices db)]
    (if (= (length available) 0)
        {:fx [{:type :terminal/read}]}
        (let [current (misa.request-options.value db option-name)
              index (accumulate [found 0 i value (ipairs available)
                                 &until (> found 0)]
                      (if (= value current) i 0))
              value (. available (+ (% index (length available)) 1))]
          {:fx [{:event {:name option-name
                         :type :request-options/select
                         : value}
                 :type :dispatch}
                {:type :terminal/read}]}))))

{: choices
 : complete-effort-select
 : on-effort-cycle
 : on-effort-select
 : option-name
 : selected-effort-select
 : unavailable}
