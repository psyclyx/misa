(local definitions (require :misa.definitions))

;; Shared click and keybinding presentation for actions, wherever they appear.
(fn components-buttons [actions context]
  "Render button descriptors within the supplied dimensions."
  (let [spans []]
    (each [index action (ipairs (or actions []))]
      (let [key (or action.key
                    (and action.binding misa.keybindings misa.keybindings.hint
                         (misa.keybindings.hint action.binding.context
                                                action.binding.action)))
            target (when (not action.disabled)
                     (if context.action_token
                         (context.action_token action)
                         (misa.dialogs.action-token context action)))]
        (when (> index 1)
          (table.insert spans {:text "   " :style :plain}))
        (let [rendered (if key
                           (misa.keybindings.reference [{:key key
                                                         :label (or action.label
                                                                    action.id)
                                                         :action target}])
                           [{:text (or action.label action.id)
                             :style :label
                             :action target}])]
          (each [_ part (ipairs rendered)]
            (when action.disabled (set part.style :dim)
              (set part.action nil))
            (table.insert spans part)))))
    spans))

(fn build []
  "Build the declarations for component buttons."
  (definitions.build :component.buttons
    [{:catalog :services :id :components.buttons :value components-buttons}]
    {:requirements {:component.buttons [:keybindings.reference]}}))

{:build build}
