(local implementation (require :misa.usage.dialog))

{:keybindings {:usage/codex-reset {:context :usage
                                   :action :codex-reset
                                   :default [:r]}
               :usage/extra-manage {:context :usage
                                    :action :extra-manage
                                    :default [:e]}}
 :actions {:usage.open {:id :usage.open
                        :label "Show usage"
                        :event {:type :usage/open}
                        :available (fn [db] (not db.dialog))}}
 :commands {:/usage {:choice_purpose :command
                     :description "Show token and subscription usage"
                     :event :usage/open
                     :name :/usage}}
 :events {:usage/usage/open {:event :usage/open
                             :handler implementation.open-dashboard
                             :priority 55000}
          :usage/usage/updated {:event :usage/updated
                                :handler implementation.update
                                :priority 55000}
          :usage/usage/tick {:event :usage/tick
                             :handler implementation.tick
                             :priority 55000}
          :usage/usage/action {:event :usage/action
                               :handler implementation.invoke-action
                               :priority 55000}}}
