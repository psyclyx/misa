;; Image acquisition policy. Decoding and clipboard I/O belong to native workers;

;; successful acquisition supplies an ordinary attachment to the draft owner.

{:setup (fn [context]
          (local config (or (. (or context.config {}) :images) {}))

          (fn state [db]
            (set db.images (or db.images {:next_id 0 :pending {}}))
            db.images)

          (misa.reg_interceptor {:before (fn [tx]
                                           (when (and tx.db.images
                                                      (not= (next tx.db.images.pending)
                                                            nil))
                                             (if (= tx.event.type
                                                    :agent/completed)
                                                 (set tx.event.keep_alive true)
                                                 (or (= tx.event.type
                                                        :editor/steer)
                                                     (and (= tx.event.type
                                                             :terminal/input)
                                                          (= tx.event.kind
                                                             :enter)))
                                                 (set tx.event
                                                      {:type :ui/redraw})))
                                           tx)
                                 :id :images/pending})
          (misa.reg_action {:available (fn [db]
                                         (and (not db.dialog) (not db.picker)))
                            :event {:type :images/paste}
                            :id :images.paste
                            :keys [:ctrl_v]
                            :label "Paste image from clipboard"})
          (misa.reg_action {:available (fn [db]
                                         (and db.editor
                                              (> (length (or db.editor.attachments
                                                             {}))
                                                 0)))
                            :event {:type :editor/detach}
                            :id :images.remove
                            :label "Remove last draft attachment"})
          (misa.reg_command {:description "Attach a PNG or JPEG file"
                             :event :images/load
                             :name :/image})

          (fn request [db event paste]
            (local current (state db))
            (set current.next_id (+ current.next_id 1))
            (local id (.. "image:" current.next_id))
            (tset current.pending id true)
            (local effect
                   {:completion :images/loaded
                    : id
                    :type (or (and paste :image/paste) :image/load)})
            (if paste (set effect.argv config.clipboard_command)
                (set effect.path (or event.path event.arguments)))
            (if (and (not paste)
                     (or (not= (type effect.path) :string) (= effect.path "")))
                (do
                  (tset current.pending id nil)
                  {: db
                   :fx [{:event {:level :error
                                 :text "Use /image <path.png or path.jpg>"
                                 :type :transcript/harness}
                         :type :dispatch}]})
                {: db :fx [effect {:type :terminal/read}]}))

          (misa.reg_event :images/paste (fn [db event] (request db event true)))
          (misa.reg_event :images/load (fn [db event] (request db event false)))
          (misa.reg_event :images/loaded
                          (fn [db event]
                            (local current (state db))
                            (if (not (. current.pending event.id)) nil
                                (do
                                  (tset current.pending event.id nil)
                                  (if (not event.ok)
                                      {: db
                                       :fx [{:event {:level :error
                                                     :text (.. "Image: "
                                                               (tostring (or (or event.message
                                                                                 event.stderr)
                                                                             "could not load")))
                                                     :type :transcript/harness}
                                             :type :dispatch}
                                            {:type :terminal/read}]}
                                      (do
                                        (local data event.data)
                                        (local attachment
                                               {:height data.height
                                                :image_id (tonumber (event.id:match "%d+$"))
                                                :name data.name
                                                :preview data.preview
                                                :source {:data data.data
                                                         :media_type data.mime_type
                                                         :type :base64}
                                                :type :image
                                                :width data.width})
                                        {: db
                                         :fx [{:event {: attachment
                                                       :type :editor/attach}
                                               :type :dispatch}
                                              {:type :terminal/read}]}))))))
          (misa.reg_event :agent/reset
                          (fn [db]
                            (local fx {})
                            (each [id (pairs (. (state db) :pending))]
                              (tset fx (+ (length fx) 1)
                                    {: id :type :operation/cancel}))
                            (set db.images.pending {})
                            {: db : fx}))
          nil)}

