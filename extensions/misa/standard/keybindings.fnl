(local keys (require :misa.keybindings))
{:services {:keybindings.action (fn [context-name event]
                                  "Resolve an input event in a binding context."
                                  (keys.resolve-action (misa.keybindings.all)
                                                       context-name event))
            :keybindings.hint (fn [context-name action]
                                "Return the first key for an action."
                                (keys.hint (misa.keybindings.all) context-name
                                           action))
            :keybindings.tokens keys.tokens
            :keybindings.text keys.text
            :keybindings.render keys.render
            :keybindings.reference keys.reference}}
