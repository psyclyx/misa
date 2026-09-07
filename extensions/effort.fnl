;; Reasoning-effort affordances over the generic request-options policy.

(local option-name :reasoning_effort)

(fn choices [db]
  (or (and misa.request_option_choices
           (misa.request_option_choices db option-name)) {}))

(fn unavailable [db]
  (let [model (or (and db.models db.models.selected) "selected model")]
    {     :fx [{:event {:level :info
                   :problem {:code :unsupported
                             :kind :request_option
                             : model
                             :option option-name}
                   :text (.. "reasoning effort is not supported by " model)
                   :type :transcript/harness}
           :type :dispatch}
          {:type :terminal/read}]}))

{:setup (fn []
          (local setup-fx [])
          (assert misa.request_option_choices
                  "effort requires request_options first")
          (table.insert setup-fx
                        {:type :register/keybinding
                         :value {:action :cycle_effort
                                 :context :global
                                 :default [:alt+f]}})
          (when (misa.has_setup_effect :register/indicator)
            (table.insert setup-fx
                          {:type :register/indicator
                           :value {:hotkey {:action :cycle_effort
                                            :context :global}
                                   :icon "◈"
                                   :id :effort
                                   :label :effort
                                   :value (fn [db]
                                            (misa.request_option_value db
                                                                       option-name))}}))
          (table.insert setup-fx
                        {:type :register/command
                         :value {:choice_purpose :command
                                 :complete (fn [_ db]
                                             (local result {})
                                             (each [_ value (ipairs (choices db))]
                                               (tset result
                                                     (+ (length result) 1)
                                                     {:label (tostring value)
                                                      :value (tostring value)}))
                                             result)
                                 :description "Choose model reasoning effort"
                                 :event :effort/select
                                 :name :/effort
                                 :preference_scope :request-options/effort
                                 :selected (fn [db]
                                             (misa.request_option_value db
                                                                        option-name))}})
          ;; Intercept the shared command-choice transaction when effort is unsupported.
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (local event (if (and (= tx.event.type :choices/command-open)
                                                                 (= tx.event.command :/effort)
                                                                 (= (length (choices tx.db)) 0))
                                                            {:type :effort/unsupported}
                                                            (and (= tx.event.type :terminal/input) misa.keybinding_action
                                                                 (= (misa.keybinding_action :global tx.event) :cycle_effort))
                                                            {:type :effort/cycle}
                                                            tx.event))
                                           (misa.patch tx {:event (misa.replace event)}))
                                 :id :effort/input}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:binding {:action :cycle_effort
                                           :context :global}
                                 :event {:type :effort/cycle}
                                 :id :effort.cycle
                                 :label "Cycle reasoning effort"}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :effort/unsupported
                         :handler (fn [db] (unavailable db))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :effort/select
                         :handler (fn [db event]
                                    (local available (choices db))
                                    (if (= (length available) 0)
                                        (unavailable db)
                                        (do
                                          (local requested
                                                 (or (and (= (type event.arguments)
                                                             :string)
                                                          (event.arguments:match "^%s*(%S+)%s*$"))
                                                     nil))
                                          (local found (accumulate [selected nil _ value (ipairs available) &until selected]
                                                         (when (= (tostring value) requested) {: value})))
                                          (assert found "unsupported reasoning effort for the selected model")
                                          {:fx [{:type :dispatch :event {:type :request-options/select :name option-name :value found.value}}
                                                {:type :terminal/read}]})))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :effort/cycle
                         :handler (fn [db]
                                    (local available (choices db))
                                    (if (= (length available) 0)
                                        {:fx [{:type :terminal/read}]}
                                        (do
                                          (local current
                                                 (misa.request_option_value db
                                                                            option-name))
                                          (var index 0)
                                          (each [i value (ipairs available)]
                                            (when (= value current)
                                              (set index i)
                                              (lua :break)))
                                          (local value
                                                 (. available
                                                    (+ (% index
                                                          (length available))
                                                       1)))
                                          {                                           :fx [{:event {:name option-name
                                                         :type :request-options/select
                                                         : value}
                                                 :type :dispatch}
                                                {:type :terminal/read}]})))})
          {:fx setup-fx})}
