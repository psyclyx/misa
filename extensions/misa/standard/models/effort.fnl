(local {: choices
        : complete-effort-select
        : on-effort-cycle
        : on-effort-select
        : option-name
        : selected-effort-select
        : unavailable} (require :misa.models.effort))

{:actions {:effort.cycle {:binding {:action :cycle_effort :context :global}
                          :event {:type :effort/cycle}
                          :id :effort.cycle
                          :label "Cycle reasoning effort"}}
 :commands {:/effort {:choice_purpose :command
                      :choice_available (fn [db]
                                          (> (length (choices db)) 0))
                      :choice_unavailable :effort/unsupported
                      :complete complete-effort-select
                      :description "Choose model reasoning effort"
                      :event :effort/select
                      :name :/effort
                      :preference_scope :request-options/effort
                      :selected selected-effort-select}}
 :events {:effort/effort/unsupported {:event :effort/unsupported
                                      :handler (fn [db] (unavailable db))
                                      :priority 62000}
          :effort/effort/select {:event :effort/select
                                 :handler on-effort-select
                                 :priority 62000}
          :effort/effort/cycle {:event :effort/cycle
                                :handler on-effort-cycle
                                :priority 62000}}
 :indicators {:effort {:hotkey {:action :cycle_effort :context :global}
                       :icon "◈"
                       :id :effort
                       :label :effort
                       :query [:request-options/indicator option-name]}}
 :keybindings {:global/cycle_effort {:action :cycle_effort
                                     :context :global
                                     :default [:alt+f]}}
 :requirements {:effort [:request-options.choices]}}
