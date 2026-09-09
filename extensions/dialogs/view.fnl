(local definitions (require :misa.definitions))

;; Compose replaceable data and dialog components without storing rendered UI.
(fn projection [db cofx]
  "Build the dialog component model and constrained layout."
  (local state db.dialog)
  (when state
    (local context {:available_lines (or cofx.available_lines
                                         cofx.terminal.lines)
                    :columns cofx.terminal.columns})
    (local actions (misa.dialogs.buttons state))
    (var model (misa.patch state {:actions (misa.replace actions)}))
    (if state.confirmation
        (set model (misa.patch model
                               {:title state.confirmation.title
                                :message state.confirmation.message
                                :input_enabled false
                                :url misa.delete
                                :code misa.delete
                                :progress misa.delete
                                :input_error misa.delete
                                :hints (misa.replace [])
                                :scroll 0}))
        state.sections
        (set model
             (misa.patch model
                         {:content {:role :data
                                    :model {:sections state.sections
                                            :actions state.actions
                                            :id state.id
                                            :correlation state.correlation}}})))
    (local rendered (misa.components.render db :dialog model context))
    (set rendered.priority 30)
    rendered))

(fn []
  "Build the declarations for dialog view."
  (definitions :dialog_view
    [{:catalog :services :id :dialogs.layout :value projection}
     {:catalog :view-layers :id :dialog :value {:handler projection}}]
    {:requirements {:dialog_view [:dialogs.enabled?]}}))
