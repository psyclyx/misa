(local definitions (require :misa.definitions))

;; Optional pending-prompt presentation, separate from submission scheduling.

(fn build []
  "Build the declarations for queue view."
  (definitions.build :queue_view
    [{:catalog :components
      :id :default.pending-prompt
      :value {:render (fn [model context]
                        (let [text (.. (: model.pending :gsub "\n" " ↵ ")
                                       (if (> model.attachment_count 0)
                                           (.. "  [" model.attachment_count
                                               " image(s)]")
                                           ""))]
                          {:lines [{:spans [{:style :label :text "Queued  "}
                                            {:action :queue.edit
                                             :style :keybinding
                                             :text :edit}
                                            {:style :plain :text " · "}
                                            {:action :queue.steer
                                             :style :keybinding
                                             :text "send now"}]}
                                   {:spans [{:style :dim
                                             :text (misa.layout.clip text
                                                                     context.columns)}]}]}))}}
     {:catalog :view-layers
      :id :pending-prompt
      :value {:handler (fn [db cofx]
                         (let [queue db.queue]
                           (if (or (not queue)
                                   (and (= queue.pending "")
                                        (= (length (or queue.attachments {})) 0)))
                               nil
                               (let [rendered (misa.components.render db
                                                                      :pending-prompt
                                                                      {:pending (or queue.pending
                                                                                    "")
                                                                       :attachment_count (length (or queue.attachments
                                                                                                     []))}
                                                                      {:columns cofx.terminal.columns})]
                                 {:dock :input :lines rendered.lines}))))}}]
    {}))

{: build}
