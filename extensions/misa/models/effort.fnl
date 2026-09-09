(local definitions (require :misa.definitions))

;; Reasoning-effort affordances over the generic request-options policy.

(local option-name :reasoning_effort)

(fn choices [db]
  (or (and misa.request-options misa.request-options.choices
           (misa.request-options.choices db option-name)) {}))

(fn unavailable [db]
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

(fn build []
  "Build the declarations for effort."
  (let [declarations []]
    (table.insert declarations
                  (let [definition {:action :cycle_effort
                                    :context :global
                                    :default [:alt+f]}]
                    {:catalog :keybindings
                     :id (.. (. definition :context) "/" (. definition :action))
                     :value definition}))
    (do
      (table.insert declarations
                    (let [definition {:hotkey {:action :cycle_effort
                                               :context :global}
                                      :icon "◈"
                                      :id :effort
                                      :label :effort
                                      :query [:request-options/indicator
                                              option-name]}]
                      {:catalog :indicators
                       :id (. definition :id)
                       :value definition})))
    (table.insert declarations
                  (let [definition {:choice_purpose :command
                                    :choice_available (fn [db]
                                                        (> (length (choices db))
                                                           0))
                                    :choice_unavailable :effort/unsupported
                                    :complete (fn [_ db]
                                                (let [result {}]
                                                  (each [_ value (ipairs (choices db))]
                                                    (tset result
                                                          (+ (length result) 1)
                                                          {:label (tostring value)
                                                           :value (tostring value)}))
                                                  result))
                                    :description "Choose model reasoning effort"
                                    :event :effort/select
                                    :name :/effort
                                    :preference_scope :request-options/effort
                                    :selected (fn [db]
                                                (misa.request-options.value db
                                                                            option-name))}]
                    {:catalog :commands
                     :id (. definition :name)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:binding {:action :cycle_effort
                                              :context :global}
                                    :event {:type :effort/cycle}
                                    :id :effort.cycle
                                    :label "Cycle reasoning effort"}]
                    {:catalog :actions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  {:catalog :events
                   :value {:event :effort/unsupported
                           :handler (fn [db] (unavailable db))}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :effort/select
                           :handler (fn [db event]
                                      (let [available (choices db)]
                                        (if (= (length available) 0)
                                            (unavailable db)
                                            (do
                                              (let [requested (or (and (= (type event.arguments)
                                                                          :string)
                                                                       (event.arguments:match "^%s*(%S+)%s*$"))
                                                                  nil)
                                                    found (accumulate [selected nil _ value (ipairs available)
                                                                       &until selected]
                                                            (when (= (tostring value)
                                                                     requested)
                                                              {: value}))]
                                                (assert found
                                                        "unsupported reasoning effort for the selected model")
                                                {:fx [{:type :dispatch
                                                       :event {:type :request-options/select
                                                               :name option-name
                                                               :value found.value}}
                                                      {:type :terminal/read}]})))))}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :effort/cycle
                           :handler (fn [db]
                                      (let [available (choices db)]
                                        (if (= (length available) 0)
                                            {:fx [{:type :terminal/read}]}
                                            (do
                                              (let [current (misa.request-options.value db
                                                                                        option-name)]
                                                (var index 0)
                                                (each [i value (ipairs available)]
                                                  (when (= value current)
                                                    (set index i)
                                                    (lua :break)))
                                                (let [value (. available
                                                               (+ (% index
                                                                     (length available))
                                                                  1))]
                                                  {:fx [{:event {:name option-name
                                                                 :type :request-options/select
                                                                 : value}
                                                         :type :dispatch}
                                                        {:type :terminal/read}]}))))))}})
    (definitions.build :effort
      declarations
      {:requirements {:effort [:request-options.choices]}})))

{: build}
