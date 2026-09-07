;; Optional pending-prompt presentation, separate from submission scheduling.

{:setup (fn []
          {:fx [{:type :register/component
                 :id :default.pending-prompt
                 :value {:render (fn [model context]
                                   (local text (.. (: model.pending :gsub "\n" " ↵ ")
                                                   (if (> model.attachment_count 0)
                                                       (.. "  [" model.attachment_count " image(s)]")
                                                       "")))
                                   {:lines [{:spans [{:style :label :text "Queued  "}
                                                    {:action :queue.edit :style :keybinding :text :edit}
                                                    {:style :plain :text " · "}
                                                    {:action :queue.steer :style :keybinding :text "send now"}]}
                                            {:spans [{:style :dim
                                                      :text (misa.layout.clip text context.columns)}]}]})}}
                {:type :register/view-layer
                 :id :pending-prompt
                 :handler (fn [db cofx]
                            (local queue db.queue)
                            (if (or (not queue)
                                    (and (= queue.pending "")
                                         (= (length (or queue.attachments {}))
                                            0)))
                                nil
                                (let [rendered (misa.render_component db :pending-prompt
                                                                     {:pending (or queue.pending "")
                                                                      :attachment_count (length (or queue.attachments []))}
                                                                     {:columns cofx.terminal.columns})]
                                  {:dock :input :lines rendered.lines})))}]})}
