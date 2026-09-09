;; Image acquisition policy. Decoding and clipboard I/O belong to native workers;
;; successful acquisition supplies an ordinary attachment to the draft owner.

(fn state [db]
  (or db.images {:next_id 0 :pending {}}))

(fn updated [current patch fx]
  {:patch {:images (misa.replace (misa.patch current patch))} : fx})

(fn compute-images-lifecycle [inputs]
  "Report whether image loading should hold editor exit."
  (let [pending (. inputs 1)
        acquiring (and (not= pending nil) (not= (next pending) nil))]
    {:hold_exit acquiring :block_draft acquiring}))

(fn available-images-remove [db]
  "Return whether the draft has a removable image."
  (and db.editor (> (length (or db.editor.attachments {})) 0)))

(fn on-images-loaded [db event]
  "Attach a decoded image or report its loading error."
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
  "Cancel pending image requests when the agent resets."
  (let [fx {}]
    (each [id (pairs (. (state db) :pending))]
      (tset fx (+ (length fx) 1) {: id :type :operation/cancel}))
    (updated (state db) {:pending (misa.replace {})} fx)))

(fn request [config db event paste]
  "Request image loading or clipboard paste with explicit settings."
  (let [config (or config.images {})]
    (let [current (state db)
          next-id (+ current.next_id 1)
          id (.. "image:" next-id)
          effect {:completion :images/loaded
                  : id
                  :type (if paste :image/paste :image/load)
                  :argv (when paste config.clipboard_command)
                  :path (when (not paste) (or event.path event.arguments))}]
      (if (and (not paste) (or (not= (type effect.path) :string)
                               (= effect.path "")))
          (updated current {:next_id next-id}
                   [{:event {:level :error
                             :text "Use /image <path.png or path.jpg>"
                             :type :transcript/harness}
                     :type :dispatch}])
          (updated current {:next_id next-id :pending {id true}}
                   [effect {:type :terminal/read}])))))

{:available-images-remove available-images-remove
 :compute-images-lifecycle compute-images-lifecycle
 :on-agent-reset on-agent-reset
 :on-images-loaded on-images-loaded
 :request request}
