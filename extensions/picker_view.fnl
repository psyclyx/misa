;; Projection adapter: choice_layout is the sole geometry owner.

{:setup (fn []
          (assert misa.choice_picker_layout
                  "picker_view requires choice_layout")
          (misa.reg_view_layer :picker
                               (fn [db cofx]
                                 (local picker db.picker)
                                 (if (not picker) nil
                                     (do
                                       (local geometry
                                              (misa.choice_picker_layout picker.session
                                                                         db
                                                                         {:available_lines cofx.available_lines
                                                                          :columns cofx.terminal.columns
                                                                          :lines cofx.terminal.lines}))
                                       (local rendered
                                              (misa.render_component db :picker
                                                                     geometry
                                                                     {:available_lines geometry.available_lines
                                                                      :columns cofx.terminal.columns}))
                                       (set rendered.priority 20)
                                       rendered))))
          nil)}

