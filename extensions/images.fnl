;; Image acquisition policy. Decoding and clipboard I/O belong to native workers;

;; successful acquisition supplies an ordinary attachment to the draft owner.

{:setup (fn [context]
          (local setup-fx [])
          (local config (or (. (or context.config {}) :images) {}))

          (fn state [db]
            (or db.images {:next_id 0 :pending {}}))

          (fn updated [current patch fx]
            {:patch {:images (misa.replace (misa.patch current patch))} : fx})

          (table.insert setup-fx
                        {:type :register/sub
                         :value {:id :images/lifecycle :inputs [[:db/path :images :pending]]
                                 :compute (fn [inputs]
                                            (local pending (. inputs 1))
                                            (local acquiring (and (not= pending nil) (not= (next pending) nil)))
                                            {:hold_exit acquiring :block_draft acquiring})}})
          (table.insert setup-fx
                        {:type :register/service :name :editor_lifecycle.images :value [:images/lifecycle]})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:available (fn [db]
                                              (and (not db.dialog)
                                                   (not db.picker)))
                                 :event {:type :images/paste}
                                 :id :images.paste
                                 :keys [:ctrl_v]
                                 :label "Paste image from clipboard"}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:available (fn [db]
                                              (and db.editor
                                                   (> (length (or db.editor.attachments
                                                                  {}))
                                                      0)))
                                 :event {:type :editor/detach}
                                 :id :images.remove
                                 :label "Remove last draft attachment"}})
          (table.insert setup-fx
                        {:type :register/command
                         :value {:description "Attach a PNG or JPEG file"
                                 :event :images/load
                                 :name :/image}})

          (fn request [db event paste]
            (local current (state db))
            (local next-id (+ current.next_id 1))
            (local id (.. "image:" next-id))
            (local effect
                   {:completion :images/loaded
                    : id
                    :type (if paste :image/paste :image/load)
                    :argv (when paste config.clipboard_command)
                    :path (when (not paste) (or event.path event.arguments))})
            (if (and (not paste)
                     (or (not= (type effect.path) :string) (= effect.path "")))
                (updated current {:next_id next-id}
                         [{:event {:level :error
                                 :text "Use /image <path.png or path.jpg>"
                                 :type :transcript/harness}
                         :type :dispatch}])
                (updated current {:next_id next-id :pending {id true}}
                         [effect {:type :terminal/read}])))

          (table.insert setup-fx
                        {:type :register/event
                         :name :images/paste
                         :handler (fn [db event] (request db event true))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :images/load
                         :handler (fn [db event] (request db event false))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :images/loaded
                         :handler (fn [db event]
                                    (local current (state db))
                                    (if (not (. current.pending event.id)) nil
                                        (do
                                          (local patch {:pending {event.id misa.delete}})
                                          (if (not event.ok)
                                              (updated current patch
                                                       [{:event {:level :error
                                                             :text (.. "Image: "
                                                                       (tostring (or (or event.message
                                                                                         event.stderr)
                                                                                     "could not load")))
                                                             :type :transcript/harness}
                                                     :type :dispatch}
                                                    {:type :terminal/read}])
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
                                                (updated current patch
                                                         [{:event {: attachment
                                                               :type :editor/attach}
                                                       :type :dispatch}
                                                      {:type :terminal/read}]))))))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :agent/reset
                         :handler (fn [db]
                                    (local fx {})
                                    (each [id (pairs (. (state db) :pending))]
                                      (tset fx (+ (length fx) 1)
                                            {: id :type :operation/cancel}))
                                    (updated (state db) {:pending (misa.replace {})} fx))})
          {:fx setup-fx})}
