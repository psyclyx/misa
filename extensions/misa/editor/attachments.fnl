(local definitions (require :misa.definitions))

;; Attachment composition is independent of acquisition and submission policy.

(fn build []
  "Build the declarations for attachments."
  (definitions.build :attachments
    [{:catalog :components
      :id :default.attachment-controls
      :value {:render (fn [model]
                        {:lines (icollect [_ line (ipairs [{:visible model.pending
                                                            :spans [{:style :pending
                                                                     :text "Loading image…"}]}
                                                           {:visible (> model.count
                                                                        0)
                                                            :spans [{:action :images.remove
                                                                     :style :keybinding
                                                                     :text "Remove last attachment"}]}])]
                                  (when line.visible {:spans line.spans}))})}}
     {:catalog :services
      :id :attachments.lines
      :value (fn [db items context]
               "Render attachment labels as terminal lines."
               (let [lines {}]
                 (each [_ item (ipairs (or items {}))]
                   (let [rendered (misa.components.render db
                                                          (.. :attachment.
                                                              item.type)
                                                          item context)]
                     (each [_ line (ipairs (or rendered.lines {}))]
                       (tset lines (+ (length lines) 1) line))))
                 lines))}
     {:catalog :view-layers
      :id :draft-attachments
      :value {:handler (fn [db cofx]
                         (let [items (or (. (or db.editor {}) :attachments) {})
                               pending (and db.images
                                            (not= (next db.images.pending) nil))]
                           (if (and (= (length items) 0) (not pending))
                               nil
                               (do
                                 (let [lines (misa.attachments.lines db items
                                                                     {:columns cofx.terminal.columns
                                                                      :images cofx.terminal.images
                                                                      :max_image_rows 5})
                                       controls (misa.components.render db
                                                                        :attachment-controls
                                                                        {:pending (= pending
                                                                                     true)
                                                                         :count (length items)}
                                                                        {:columns cofx.terminal.columns})]
                                   (each [_ line (ipairs controls.lines)]
                                     (table.insert lines line))
                                   {:dock :input : lines})))))}}]
    {}))

{: build}
