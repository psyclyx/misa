(local definitions (require :misa.definitions))

;; Projection adapter: choices.layout is the sole geometry owner.

(fn []
  "Build the declarations for picker view."
  (local declarations [])
  (table.insert declarations
                {:catalog :view-layers
                 :id :picker
                 :value {:handler (fn [db cofx]
                                    (local picker db.picker)
                                    (if (not picker) nil
                                        (do
                                          (local geometry
                                                 (misa.choices.picker-layout picker.session
                                                                             db
                                                                             {:available_lines cofx.available_lines
                                                                              :columns cofx.terminal.columns
                                                                              :lines cofx.terminal.lines}))
                                          (local rendered
                                                 (misa.components.render db
                                                                         :picker
                                                                         geometry
                                                                         {:available_lines geometry.available_lines
                                                                          :columns cofx.terminal.columns}))
                                          (set rendered.priority 20)
                                          rendered)))}})
  (definitions :picker_view
    declarations
    {:requirements {:picker_view [:choices.picker-layout]}}))
