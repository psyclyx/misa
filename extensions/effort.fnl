;; Reasoning-effort affordances over the generic request-options policy.

(local option-name :reasoning_effort)

(fn choices [db]
  (or (and misa.request_option_choices
           (misa.request_option_choices db option-name)) {}))

(fn unavailable [db]
  (let [model (or (and db.models db.models.selected) "selected model")]
    {: db
     :fx [{:event {:level :info
                   :problem {:code :unsupported
                             :kind :request_option
                             : model
                             :option option-name}
                   :text (.. "reasoning effort is not supported by " model)
                   :type :transcript/harness}
           :type :dispatch}
          {:type :terminal/read}]}))

{:setup (fn []
          (assert misa.request_option_choices
                  "effort requires request_options first")
          (misa.reg_keybinding {:action :cycle_effort
                                :context :global
                                :default [:alt+f]})
          (when misa.reg_indicator
            (misa.reg_indicator {:hotkey {:action :cycle_effort
                                          :context :global}
                                 :icon "◈"
                                 :id :effort
                                 :label :effort
                                 :value (fn [db]
                                          (misa.request_option_value db
                                                                     option-name))}))
          (misa.reg_command {:choice_purpose :command
                             :complete (fn [_ db]
                                         (local result {})
                                         (each [_ value (ipairs (choices db))]
                                           (tset result (+ (length result) 1)
                                                 {:label (tostring value)
                                                  :value (tostring value)}))
                                         result)
                             :description "Choose model reasoning effort"
                             :event :effort/select
                             :name :/effort
                             :preference_scope :request-options/effort
                             :selected (fn [db]
                                         (misa.request_option_value db
                                                                    option-name))})
          ;; Intercept the shared command-choice transaction when effort is unsupported.
          (misa.reg_interceptor {:before (fn [tx]
                                           (if (and (and (= tx.event.type
                                                            :choices/command-open)
                                                         (= tx.event.command
                                                            :/effort))
                                                    (= (length (choices tx.db))
                                                       0))
                                               (set tx.event
                                                    {:type :effort/unsupported})
                                               (and (and (= tx.event.type
                                                            :terminal/input)
                                                         misa.keybinding_action)
                                                    (= (misa.keybinding_action :global
                                                                               tx.event)
                                                       :cycle_effort))
                                               (set tx.event
                                                    {:type :effort/cycle}))
                                           tx)
                                 :id :effort/input})
          (misa.reg_action {:binding {:action :cycle_effort :context :global}
                            :event {:type :effort/cycle}
                            :id :effort.cycle
                            :label "Cycle reasoning effort"})
          (misa.reg_event :effort/unsupported (fn [db] (unavailable db)))
          (misa.reg_event :effort/select
                          (fn [db event]
                            (local available (choices db))
                            (if (= (length available) 0) (unavailable db)
                                (do
                                  (local requested
                                         (or (and (= (type event.arguments)
                                                     :string)
                                                  (event.arguments:match "^%s*(%S+)%s*$"))
                                             nil))
                                  (each [_ value (ipairs available)]
                                    (when (= (tostring value) requested)
                                      (let [___antifnl_rtn_1___ {: db
                                                                 :fx [{:event {:name option-name
                                                                               :type :request-options/select
                                                                               : value}
                                                                       :type :dispatch}
                                                                      {:type :terminal/read}]}]
                                        (lua "return ___antifnl_rtn_1___"))))
                                  (error "unsupported reasoning effort for the selected model")
                                  nil))))
          (misa.reg_event :effort/cycle
                          (fn [db]
                            (local available (choices db))
                            (if (= (length available) 0)
                                {: db :fx [{:type :terminal/read}]}
                                (do
                                  (local current
                                         (misa.request_option_value db
                                                                    option-name))
                                  (var index 0)
                                  (each [i value (ipairs available)]
                                    (when (= value current) (set index i)
                                      (lua :break)))
                                  (local value
                                         (. available
                                            (+ (% index (length available)) 1)))
                                  {: db
                                   :fx [{:event {:name option-name
                                                 :type :request-options/select
                                                 : value}
                                         :type :dispatch}
                                        {:type :terminal/read}]}))))
          nil)}

