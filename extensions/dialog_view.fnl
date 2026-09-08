;; Compose replaceable data and dialog components without storing rendered UI.
(fn projection [db cofx]
  (local state db.dialog)
  (when state
    (local context {:available_lines (or cofx.available_lines cofx.terminal.lines)
                    :columns cofx.terminal.columns})
    (local actions (misa.dialog_buttons state))
    (var model (misa.patch state {:actions (misa.replace actions)}))
    (if state.confirmation
        (set model (misa.patch model {:title state.confirmation.title
                                     :message state.confirmation.message :input_enabled false
                                     :url misa.delete :code misa.delete :progress misa.delete
                                     :input_error misa.delete :hints (misa.replace []) :scroll 0}))
        state.sections
        (let [data ((. (misa.component db :data) :render)
                    {:sections state.sections :actions state.actions :id state.id :correlation state.correlation}
                    {:columns (math.max 1 (- context.columns 2))})]
          (set model (misa.patch model {:content data.lines}))))
    (local rendered (misa.render_component db :dialog model context))
    (set rendered.priority 30)
    rendered))

{:setup (fn []
          (assert misa.dialogs "dialog_view requires dialogs")
          {:fx [{:type :register/service :name :dialog_projection :value projection}
                {:type :register/view-layer :id :dialog :handler projection}]})}
