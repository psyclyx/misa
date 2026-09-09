;; Attachment composition is independent of acquisition and submission policy.

(fn on-draft-attachments [db cofx]
  "Project controls for the current draft attachments."
  (let [items (or (. (or db.editor {}) :attachments) {})
        pending (and db.images (not= (next db.images.pending) nil))]
    (if (and (= (length items) 0) (not pending))
        nil
        (let [lines (misa.attachments.lines db items
                                            {:columns cofx.terminal.columns
                                             :images cofx.terminal.images
                                             :max_image_rows 5})
              controls (misa.components.render db :attachment-controls
                                               {:pending (= pending true)
                                                :count (length items)}
                                               {:columns cofx.terminal.columns})]
          (each [_ line (ipairs controls.lines)]
            (table.insert lines line))
          {:dock :input : lines}))))

(fn attachments-lines [db items context]
  "Render attachment labels as terminal lines."
  (let [lines {}]
    (each [_ item (ipairs (or items {}))]
      (let [rendered (misa.components.render db (.. :attachment. item.type)
                                             item context)]
        (each [_ line (ipairs (or rendered.lines {}))]
          (tset lines (+ (length lines) 1) line))))
    lines))

(fn render-controls [model]
  "Render attachment controls."
  {:lines (icollect [_ line (ipairs [{:visible model.pending
                                      :spans [{:style :pending
                                               :text "Loading image…"}]}
                                     {:visible (> model.count 0)
                                      :spans [{:action :images.remove
                                               :style :keybinding
                                               :text "Remove last attachment"}]}])]
            (when line.visible {:spans line.spans}))})

{:attachments-lines attachments-lines
 :on-draft-attachments on-draft-attachments
 :render-controls render-controls}
