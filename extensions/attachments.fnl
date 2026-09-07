;; Attachment composition is independent of acquisition and submission policy.

{:setup (fn []
          {:fx [{:type :register/component
                 :id :default.attachment-controls
                 :value {:render (fn [model]
                                   {:lines (icollect [_ line (ipairs
                                              [{:visible model.pending
                                                :spans [{:style :pending :text "Loading image…"}]}
                                               {:visible (> model.count 0)
                                                :spans [{:action :images.remove :style :keybinding
                                                         :text "Remove last attachment"}]}])]
                                             (when line.visible {:spans line.spans}))})}}
                {:type :register/service
                 :name :attachment_lines
                 :value (fn [db items context]
                          (local lines {})
                          (each [_ item (ipairs (or items {}))]
                            (local rendered
                                   (misa.render_component db
                                                          (.. :attachment.
                                                              item.type)
                                                          item context))
                            (each [_ line (ipairs (or rendered.lines {}))]
                              (tset lines (+ (length lines) 1) line)))
                          lines)}
                {:type :register/view-layer
                 :id :draft-attachments
                 :handler (fn [db cofx]
                            (local items
                                   (or (. (or db.editor {}) :attachments) {}))
                            (local pending
                                   (and db.images
                                        (not= (next db.images.pending) nil)))
                            (if (and (= (length items) 0) (not pending))
                                nil
                                (do
                                  (local lines
                                         (misa.attachment_lines db items
                                                                {:columns cofx.terminal.columns
                                                                 :images cofx.terminal.images
                                                                 :max_image_rows 5}))
                                  (local controls
                                         (misa.render_component db :attachment-controls
                                                                {:pending (= pending true)
                                                                 :count (length items)}
                                                                {:columns cofx.terminal.columns}))
                                  (each [_ line (ipairs controls.lines)]
                                    (table.insert lines line))
                                  {:dock :input : lines})))}]})}
