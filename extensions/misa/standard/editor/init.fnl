(local editor (require :misa.editor))

(fn start [db event cofx]
  (editor.app-start cofx.config db event cofx))

(fn read-input []
  {:fx [{:type :terminal/read}]})

{:editor-edits {:text editor.insert-text
                :backspace editor.backspace
                :arrow_left (fn [draft]
                              (editor.with-text draft draft.text
                                (editor.previous-cursor draft.text draft.cursor)))
                :arrow_right (fn [draft]
                               (editor.with-text draft draft.text
                                 (editor.next-cursor draft.text draft.cursor)))
                :ctrl_c editor.emptied
                :ctrl_d editor.ctrl-d}
 :events {:editor/agent/completed {:event :agent/completed
                                   :handler (fn [db event cofx]
                                              (editor.account-transition editor.agent-completed
                                                                         db
                                                                         event
                                                                         cofx))
                                   :priority 67000}
          :editor/agent/status {:event :agent/status
                                :handler (fn [db event cofx]
                                           (editor.account-transition editor.agent-status
                                                                      db event
                                                                      cofx))
                                :priority 67000}
          :editor/agent/unavailable {:event :agent/unavailable
                                     :handler (fn [db event cofx]
                                                (editor.account-transition editor.agent-unavailable
                                                                           db
                                                                           event
                                                                           cofx))
                                     :priority 67000}
          :editor/app/start {:event :app/start
                             :handler (fn [db event cofx]
                                        (editor.account-transition start db
                                                                   event cofx))
                             :priority 67000}
          :editor/editor/attach {:event :editor/attach
                                 :handler (fn [db event cofx]
                                            (editor.account-transition editor.attach
                                                                       db event
                                                                       cofx))
                                 :priority 67000}
          :editor/editor/choice-selected {:event :editor/choice-selected
                                          :handler (fn [db event cofx]
                                                     (editor.account-transition editor.selected
                                                                                db
                                                                                event
                                                                                cofx))
                                          :priority 67000}
          :editor/editor/completion-check {:event :editor/completion-check
                                           :handler (fn [db event cofx]
                                                      (editor.account-transition editor.editor-completion-check
                                                                                 db
                                                                                 event
                                                                                 cofx))
                                           :priority 67000}
          :editor/editor/detach {:event :editor/detach
                                 :handler (fn [db event cofx]
                                            (editor.account-transition editor.detach
                                                                       db event
                                                                       cofx))
                                 :priority 67000}
          :editor/editor/restore {:event :editor/restore
                                  :handler (fn [db event cofx]
                                             (editor.account-transition editor.restore
                                                                        db event
                                                                        cofx))
                                  :priority 67000}
          :editor/editor/steer {:event :editor/steer
                                :handler (fn [db event cofx]
                                           (editor.account-transition editor.editor-steer
                                                                      db event
                                                                      cofx))
                                :priority 67000}
          :editor/terminal/input {:event :terminal/input
                                  :handler (fn [db event cofx]
                                             (editor.account-transition editor.terminal-input
                                                                        db event
                                                                        cofx))
                                  :priority 67000}
          :editor/ui/redraw {:event :ui/redraw
                             :handler (fn [db event cofx]
                                        (editor.account-transition read-input
                                                                   db event cofx))
                             :priority 67000}}
 :projections {:editor.project-input {:inputs editor.input-model
                                      :render editor.render-editor-project-input}}
 :services {:editor.layout editor.editor-layout}
 :subscriptions {:editor/lifecycle {:id :editor/lifecycle
                                    :inputs editor.lifecycle-inputs
                                    :compute editor.compute-editor-lifecycle}}
 :validators {:editor-edits editor.validate-transition}}
