;; Replaceable projection for generic dialog state.

{:setup (fn []
          (assert misa.dialogs "dialog_view requires dialogs")
          (misa.reg_view_layer :dialog
                               (fn [db cofx]
                                 (local state db.dialog)
                                 (if (not state) nil
                                     (do
                                       (local rendered
                                              (misa.render_component db :dialog
                                                                     state
                                                                     {:available_lines (or cofx.available_lines
                                                                                           cofx.terminal.lines)
                                                                      :columns cofx.terminal.columns}))
                                       (set rendered.priority 30)
                                       rendered))))
          nil)}

