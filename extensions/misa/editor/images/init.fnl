(local definitions (require :misa.definitions))

;; Image acquisition policy. Decoding and clipboard I/O belong to native workers;
;; successful acquisition supplies an ordinary attachment to the draft owner.

(fn [context]
  "Build the declarations for images."
  (local declarations [])
  (local config (or (. (or context.config {}) :images) {}))

  (fn state [db]
    (or db.images {:next_id 0 :pending {}}))

  (fn updated [current patch fx]
    {:patch {:images (misa.replace (misa.patch current patch))} : fx})

  (table.insert declarations
                (let [definition {:id :images/lifecycle
                                  :inputs [[:db/path :images :pending]]
                                  :compute (fn [inputs]
                                             (local pending (. inputs 1))
                                             (local acquiring
                                                    (and (not= pending nil)
                                                         (not= (next pending)
                                                               nil)))
                                             {:hold_exit acquiring
                                              :block_draft acquiring})}]
                  {:catalog :subscriptions
                   :id (. definition :id)
                   :value definition}))
  (table.insert declarations
                {:catalog :services
                 :id :editor.lifecycle.images
                 :value [:images/lifecycle]})
  (table.insert declarations
                (let [definition {:available (fn [db]
                                               (and (not db.dialog)
                                                    (not db.picker)))
                                  :event {:type :images/paste}
                                  :id :images.paste
                                  :keys [:ctrl_v]
                                  :label "Paste image from clipboard"}]
                  {:catalog :actions :id (. definition :id) :value definition}))
  (table.insert declarations
                (let [definition {:available (fn [db]
                                               (and db.editor
                                                    (> (length (or db.editor.attachments
                                                                   {}))
                                                       0)))
                                  :event {:type :editor/detach}
                                  :id :images.remove
                                  :label "Remove last draft attachment"}]
                  {:catalog :actions :id (. definition :id) :value definition}))
  (table.insert declarations
                (let [definition {:description "Attach a PNG or JPEG file"
                                  :event :images/load
                                  :name :/image}]
                  {:catalog :commands
                   :id (. definition :name)
                   :value definition}))

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
    (if (and (not paste) (or (not= (type effect.path) :string)
                             (= effect.path "")))
        (updated current {:next_id next-id}
                 [{:event {:level :error
                           :text "Use /image <path.png or path.jpg>"
                           :type :transcript/harness}
                   :type :dispatch}])
        (updated current {:next_id next-id :pending {id true}}
                 [effect {:type :terminal/read}])))

  (table.insert declarations
                {:catalog :events
                 :value {:event :images/paste
                         :handler (fn [db event] (request db event true))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :images/load
                         :handler (fn [db event] (request db event false))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :images/loaded
                         :handler (fn [db event]
                                    (local current (state db))
                                    (if (not (. current.pending event.id)) nil
                                        (do
                                          (local patch
                                                 {:pending {event.id misa.delete}})
                                          (if (not event.ok)
                                              (updated current patch
                                                       [{:event {:level :error
                                                                 :text (.. "Image: "
                                                                           (tostring (or event.message
                                                                                         event.stderr
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
                                                          {:type :terminal/read}]))))))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :agent/reset
                         :handler (fn [db]
                                    (local fx {})
                                    (each [id (pairs (. (state db) :pending))]
                                      (tset fx (+ (length fx) 1)
                                            {: id :type :operation/cancel}))
                                    (updated (state db)
                                             {:pending (misa.replace {})} fx))}})
  (definitions :images declarations {}))
