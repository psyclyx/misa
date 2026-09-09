(local implementation (require :misa.actions))

{:events {:actions/ui/hover {:event :ui/hover
                             :handler implementation.hover
                             :priority 16000}
          :actions/ui/action {:event :ui/action
                              :handler implementation.invoke-action
                              :priority 16000}
          :actions/actions/open {:event :actions/open
                                 :handler implementation.open-palette
                                 :priority 16000}
          :actions/actions/selected {:event :actions/selected
                                     :handler implementation.select-action
                                     :priority 16000}}
 :keybindings {:global/action_palette {:action :action_palette
                                       :context :global
                                       :default [:f1]}}
 :actions {:actions.open {:binding {:action :action_palette :context :global}
                          :event {:type :actions/open}
                          :id :actions.open
                          :label "Open action palette / key reference"}}
 :routes {:actions/picker-palette {:id :actions/picker-palette
                                   :event :terminal/input
                                   :priority 900
                                   :context [:db/path :picker]
                                   :resolve implementation.palette-shortcut}
          :actions/input {:id :actions/input
                          :event :terminal/input
                          :priority 700
                          :context [:db/path]
                          :resolve implementation.resolve-global-input}}}
