(local definitions (require :misa.definitions))

;; Shared click and keybinding presentation for actions, wherever they appear.
(fn []
  "Build the declarations for component buttons."
  (definitions :component.buttons
    [{:catalog :services
      :id :components.buttons
      :value (fn [actions context]
               "Render button descriptors within the supplied dimensions."
               (local spans [])
               (each [index action (ipairs (or actions []))]
                 (local key
                        (or action.key
                            (and action.binding misa.keybindings
                                 misa.keybindings.hint
                                 (misa.keybindings.hint action.binding.context
                                                        action.binding.action))))
                 (local target
                        (when (not action.disabled)
                          (if context.action_token
                              (context.action_token action)
                              (misa.dialogs.action-token context action))))
                 (when (> index 1)
                   (table.insert spans {:text "   " :style :plain}))
                 (local rendered
                        (if key
                            (misa.keybindings.reference [{:key key
                                                          :label (or action.label
                                                                     action.id)
                                                          :action target}])
                            [{:text (or action.label action.id)
                              :style :label
                              :action target}]))
                 (each [_ part (ipairs rendered)]
                   (when action.disabled (set part.style :dim)
                     (set part.action nil))
                   (table.insert spans part)))
               spans)}]
    {:requirements {:component.buttons [:keybindings.reference]}}))
