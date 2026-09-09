(local {: on-picker-input
        : on-picker-open
        : on-picker-update
        : route-terminal-input} (require :misa.choices.picker))

{:events {"picker/picker/open" {:event :picker/open
                                :handler on-picker-open
                                :priority 56000}
          "picker/picker/update" {:event :picker/update
                                  :handler on-picker-update
                                  :priority 56000}
          "picker/picker/input" {:event :picker/input
                                 :handler on-picker-input
                                 :priority 56000}}
 :routes {:picker/input {:id :picker/input
                         :event :terminal/input
                         :priority 800
                         :context [:db/path :picker]
                         :resolve route-terminal-input}}
 :services {:picker.enabled? true}
 :requirements {:picker.enabled? [:choices.action
                                  :choices.picker-layout
                                  :choices.session]}}
