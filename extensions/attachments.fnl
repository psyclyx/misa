;; Attachment composition is independent of acquisition and submission policy.

{:setup (fn []
          (fn misa.attachment_lines [db items context]
            (local lines {})
            (each [_ item (ipairs (or items {}))]
              (local rendered
                     (misa.render_component db (.. :attachment. item.type) item
                                            context))
              (each [_ line (ipairs (or rendered.lines {}))]
                (tset lines (+ (length lines) 1) line)))
            lines)

          (misa.reg_view_layer :draft-attachments
                               (fn [db cofx]
                                 (local items
                                        (or (. (or db.editor {}) :attachments)
                                            {}))
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
                                       (when pending
                                         (tset lines (+ (length lines) 1)
                                               {:spans [{:style (misa.theme_style db
                                                                                  :pending)
                                                         :text "Loading image…"}]}))
                                       (when (> (length items) 0)
                                         (tset lines (+ (length lines) 1)
                                               {:spans [{:action :images.remove
                                                         :style (misa.theme_style db
                                                                                  :keybinding)
                                                         :text "Remove last attachment"}]}))
                                       {:dock :input : lines}))))
          nil)}

