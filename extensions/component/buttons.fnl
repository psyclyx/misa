;; Shared click and keybinding presentation for actions, wherever they appear.
{:setup (fn []
          (assert misa.render_keybinding_reference "component.buttons requires keybindings")
          {:fx [{:type :register/service :name :render_buttons
                 :value (fn [actions context]
                          (local spans [])
                          (each [index action (ipairs (or actions []))]
                            (local key (or action.key
                                           (and action.binding misa.keybinding_hint
                                                (misa.keybinding_hint action.binding.context action.binding.action))))
                            (local target (when (not action.disabled)
                                            (if context.action_token (context.action_token action)
                                                (misa.dialog_action_token context action))))
                            (when (> index 1) (table.insert spans {:text "   " :style :plain}))
                            (local rendered (if key
                                               (misa.render_keybinding_reference [{:key key :label (or action.label action.id) :action target}])
                                               [{:text (or action.label action.id) :style :label :action target}]))
                            (each [_ part (ipairs rendered)]
                              (when action.disabled (set part.style :dim) (set part.action nil))
                              (table.insert spans part)))
                          spans)}]})}
