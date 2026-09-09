(local definitions (require :misa.definitions))

;; Projection adapter: choices.layout is the sole geometry owner.

(fn on-picker [db cofx]
  (let [picker db.picker]
    (if (not picker) nil (let [geometry (misa.choices.picker-layout picker.session
                                                                    db
                                                                    {:available_lines cofx.available_lines
                                                                     :columns cofx.terminal.columns
                                                                     :lines cofx.terminal.lines})
                               rendered (misa.components.render db :picker
                                                                geometry
                                                                {:available_lines geometry.available_lines
                                                                 :columns cofx.terminal.columns})]
                           (set rendered.priority 20)
                           rendered))))

(fn build []
  "Build the declarations for picker view."
  (definitions.build :picker_view
    [{:catalog :view-layers :id :picker :value {:handler on-picker}}]
    {:requirements {:picker_view [:choices.picker-layout]}}))

{: build}
