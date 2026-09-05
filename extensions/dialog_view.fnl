;; Replaceable projection for generic dialog state.

{:setup (fn []
          (local setup-fx [])
          (assert misa.dialogs "dialog_view requires dialogs")
          (table.insert setup-fx
                        {:type :register/view-layer
                         :id :dialog
                         :handler (fn [db cofx]
                                    (local state db.dialog)
                                    (if (not state) nil
                                        (do
                                          (local rendered
                                                 (misa.render_component db
                                                                        :dialog
                                                                        state
                                                                        {:available_lines (or cofx.available_lines
                                                                                              cofx.terminal.lines)
                                                                         :columns cofx.terminal.columns}))
                                          (set rendered.priority 30)
                                          rendered)))})
          {:fx setup-fx})}
