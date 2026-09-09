(local dialogs (require :misa.dialogs))

{:dialog-inputs {:escape dialogs.cancel
                 :ctrl_c dialogs.cancel
                 :ctrl_d dialogs.cancel
                 :eof dialogs.cancel
                 :text dialogs.insert-text
                 :backspace dialogs.backspace
                 :enter dialogs.enter}
 :events {"dialogs/dialog/action" {:event :dialog/action
                                   :handler dialogs.on-dialog-action
                                   :priority 19000}
          :dialogs/dialog/close {:event :dialog/close
                                 :handler dialogs.close
                                 :priority 19000}
          :dialogs/dialog/input {:event :dialog/input
                                 :handler dialogs.input
                                 :priority 19000}
          :dialogs/dialog/open {:event :dialog/open
                                :handler dialogs.open
                                :priority 19000}
          :dialogs/dialog/protected-input {:event :dialog/protected-input
                                           :handler dialogs.protected-input
                                           :priority 19000}
          :dialogs/dialog/update {:event :dialog/update
                                  :handler dialogs.update
                                  :priority 19000}}
 :routes {:dialogs/action {:id :dialogs/action
                           :event :ui/action
                           :priority 1000
                           :context [:db/path :dialog]
                           :resolve dialogs.route-ui-action}
          :dialogs/input {:id :dialogs/input
                          :event :terminal/input
                          :priority 1000
                          :context [:db/path :dialog]
                          :resolve (fn [_ event]
                                     (misa.patch event {:type :dialog/input}))}}
 :services {:dialogs.action-token dialogs.token
            :dialogs.buttons dialogs.buttons
            :dialogs.enabled? true}
 :validators {:dialog-inputs dialogs.dialog-inputs}}
