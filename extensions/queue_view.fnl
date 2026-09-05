;; Optional pending-prompt presentation, separate from submission scheduling.

{:setup (fn []
          {:fx [{:type :register/view-layer
                 :id :pending-prompt
                 :handler (fn [db cofx]
                            (local queue db.queue)
                            (if (or (not queue)
                                    (and (= queue.pending "")
                                         (= (length (or queue.attachments {}))
                                            0)))
                                nil
                                (do
                                  (local lines
                                         [{:spans [{:style :label
                                                    :text "Queued  "}
                                                   {:action :queue.edit
                                                    :style :keybinding
                                                    :text :edit}
                                                   {:style :plain :text " · "}
                                                   {:action :queue.steer
                                                    :style :keybinding
                                                    :text "send now"}]}])
                                  (var text
                                       (: (or queue.pending "") :gsub "\n"
                                          " ↵ "))
                                  (when (> (length (or queue.attachments {})) 0)
                                    (set text
                                         (.. text "  ["
                                             (length queue.attachments)
                                             " image(s)]")))
                                  (tset lines (+ (length lines) 1)
                                        {:spans [{:style :dim
                                                  :text (misa.layout.clip text
                                                                          cofx.terminal.columns)}]})
                                  (each [_ line (ipairs lines)]
                                    (each [_ span (ipairs line.spans)]
                                      (set span.style
                                           (misa.theme_style db span.style))))
                                  {:dock :input : lines})))}]})}
