(local definitions (require :misa.definitions))

;; Image acquisition policy. Decoding and clipboard I/O belong to native workers;
;; successful acquisition supplies an ordinary attachment to the draft owner.

(fn state [db]
  (or db.images {:next_id 0 :pending {}}))

(fn updated [current patch fx]
  {:patch {:images (misa.replace (misa.patch current patch))} : fx})

(fn compute-images-lifecycle [inputs]
  (let [pending (. inputs 1)
        acquiring (and (not= pending nil) (not= (next pending) nil))]
    {:hold_exit acquiring :block_draft acquiring}))

(fn available-images-remove [db]
  (and db.editor (> (length (or db.editor.attachments {})) 0)))

(fn on-images-loaded [db event]
  (let [current (state db)]
    (if (not (. current.pending event.id))
        nil
        (let [patch {:pending {event.id misa.delete}}]
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
              (let [data event.data
                    attachment {:height data.height
                                :image_id (tonumber (event.id:match "%d+$"))
                                :name data.name
                                :preview data.preview
                                :source {:data data.data
                                         :media_type data.mime_type
                                         :type :base64}
                                :type :image
                                :width data.width}]
                (updated current patch
                         [{:event {: attachment :type :editor/attach}
                           :type :dispatch}
                          {:type :terminal/read}])))))))

(fn on-agent-reset [db]
  (let [fx {}]
    (each [id (pairs (. (state db) :pending))]
      (tset fx (+ (length fx) 1) {: id :type :operation/cancel}))
    (updated (state db) {:pending (misa.replace {})} fx)))

(fn build [context]
  "Build the declarations for images."
  (let [declarations []
        config (or (. (or context.config {}) :images) {})]
    (table.insert declarations
                  (let [definition {:id :images/lifecycle
                                    :inputs [[:db/path :images :pending]]
                                    :compute compute-images-lifecycle}]
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
                    {:catalog :actions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:available available-images-remove
                                    :event {:type :editor/detach}
                                    :id :images.remove
                                    :label "Remove last draft attachment"}]
                    {:catalog :actions
                     :id (. definition :id)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:description "Attach a PNG or JPEG file"
                                    :event :images/load
                                    :name :/image}]
                    {:catalog :commands
                     :id (. definition :name)
                     :value definition}))

    (fn request [db event paste]
      (let [current (state db)
            next-id (+ current.next_id 1)
            id (.. "image:" next-id)
            effect {:completion :images/loaded
                    : id
                    :type (if paste :image/paste :image/load)
                    :argv (when paste config.clipboard_command)
                    :path (when (not paste) (or event.path event.arguments))}]
        (if (and (not paste)
                 (or (not= (type effect.path) :string) (= effect.path "")))
            (updated current {:next_id next-id}
                     [{:event {:level :error
                               :text "Use /image <path.png or path.jpg>"
                               :type :transcript/harness}
                       :type :dispatch}])
            (updated current {:next_id next-id :pending {id true}}
                     [effect {:type :terminal/read}]))))

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
                   :value {:event :images/loaded :handler on-images-loaded}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :agent/reset :handler on-agent-reset}})
    (definitions.build :images declarations {})))

{: build}
