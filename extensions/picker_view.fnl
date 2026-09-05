;; Projection adapter: choice_layout is the sole geometry owner.

{:setup (fn []
          (local setup-fx [])
          (assert misa.choice_picker_layout
                  "picker_view requires choice_layout")
          (table.insert setup-fx
                        {:type :register/view-layer
                         :id :picker
                         :handler (fn [db cofx]
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
                                                 (misa.render_component db
                                                                        :picker
                                                                        geometry
                                                                        {:available_lines geometry.available_lines
                                                                         :columns cofx.terminal.columns}))
                                          (set rendered.priority 20)
                                          rendered)))})
          {:fx setup-fx})}
