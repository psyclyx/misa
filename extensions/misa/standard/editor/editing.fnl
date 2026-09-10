(local editing (require :misa.editor.editing))

{:actions {:editor.append {:available editing.available?
                           :binding {:action :append :context :editor.normal}
                           :event {:action :append :type :editing/action}
                           :id :editor.append
                           :label "Editor: append"}
           :editor.append_end {:available editing.available?
                               :binding {:action :append_end
                                         :context :editor.normal}
                               :event {:action :append_end
                                       :type :editing/action}
                               :id :editor.append_end
                               :label "Editor: append end"}
           :editor.change {:available editing.available?
                           :binding {:action :change :context :editor.normal}
                           :event {:action :change :type :editing/action}
                           :id :editor.change
                           :label "Editor: change"}
           :editor.delete {:available editing.available?
                           :binding {:action :delete :context :editor.normal}
                           :event {:action :delete :type :editing/action}
                           :id :editor.delete
                           :label "Editor: delete"}
           :editor.delete_character {:available editing.available?
                                     :binding {:action :delete_character
                                               :context :editor.normal}
                                     :event {:action :delete_character
                                             :type :editing/action}
                                     :id :editor.delete_character
                                     :label "Editor: delete character"}
           :editor.down {:available editing.available?
                         :binding {:action :down :context :editor.normal}
                         :event {:action :down :type :editing/action}
                         :id :editor.down
                         :label "Editor: down"}
           :editor.first {:available editing.available?
                          :binding {:action :first :context :editor.normal}
                          :event {:action :first :type :editing/action}
                          :id :editor.first
                          :label "Editor: first"}
           :editor.insert {:available editing.available?
                           :binding {:action :insert :context :editor.normal}
                           :event {:action :insert :type :editing/action}
                           :id :editor.insert
                           :label "Editor: insert"}
           :editor.insert_start {:available editing.available?
                                 :binding {:action :insert_start
                                           :context :editor.normal}
                                 :event {:action :insert_start
                                         :type :editing/action}
                                 :id :editor.insert_start
                                 :label "Editor: insert start"}
           :editor.last {:available editing.available?
                         :binding {:action :last :context :editor.normal}
                         :event {:action :last :type :editing/action}
                         :id :editor.last
                         :label "Editor: last"}
           :editor.left {:available editing.available?
                         :binding {:action :left :context :editor.normal}
                         :event {:action :left :type :editing/action}
                         :id :editor.left
                         :label "Editor: left"}
           :editor.line_end {:available editing.available?
                             :binding {:action :line_end
                                       :context :editor.normal}
                             :event {:action :line_end :type :editing/action}
                             :id :editor.line_end
                             :label "Editor: line end"}
           :editor.line_start {:available editing.available?
                               :binding {:action :line_start
                                         :context :editor.normal}
                               :event {:action :line_start
                                       :type :editing/action}
                               :id :editor.line_start
                               :label "Editor: line start"}
           :editor.normal {:available editing.available?
                           :binding {:action :normal :context :editor.normal}
                           :event {:action :normal :type :editing/action}
                           :id :editor.normal
                           :label "Editor: normal"}
           :editor.open_above {:available editing.available?
                               :binding {:action :open_above
                                         :context :editor.normal}
                               :event {:action :open_above
                                       :type :editing/action}
                               :id :editor.open_above
                               :label "Editor: open above"}
           :editor.open_below {:available editing.available?
                               :binding {:action :open_below
                                         :context :editor.normal}
                               :event {:action :open_below
                                       :type :editing/action}
                               :id :editor.open_below
                               :label "Editor: open below"}
           :editor.paste {:available editing.available?
                          :binding {:action :paste :context :editor.normal}
                          :event {:action :paste :type :editing/action}
                          :id :editor.paste
                          :label "Editor: paste"}
           :editor.redo {:available editing.available?
                         :binding {:action :redo :context :editor.normal}
                         :event {:action :redo :type :editing/action}
                         :id :editor.redo
                         :label "Editor: redo"}
           :editor.right {:available editing.available?
                          :binding {:action :right :context :editor.normal}
                          :event {:action :right :type :editing/action}
                          :id :editor.right
                          :label "Editor: right"}
           :editor.submit {:available editing.available?
                           :binding {:action :submit :context :editor.normal}
                           :event {:action :submit :type :editing/action}
                           :id :editor.submit
                           :label "Editor: submit"}
           :editor.undo {:available editing.available?
                         :binding {:action :undo :context :editor.normal}
                         :event {:action :undo :type :editing/action}
                         :id :editor.undo
                         :label "Editor: undo"}
           :editor.up {:available editing.available?
                       :binding {:action :up :context :editor.normal}
                       :event {:action :up :type :editing/action}
                       :id :editor.up
                       :label "Editor: up"}
           :editor.visual {:available editing.available?
                           :binding {:action :visual :context :editor.normal}
                           :event {:action :visual :type :editing/action}
                           :id :editor.visual
                           :label "Editor: visual"}
           :editor.visual_line {:available editing.available?
                                :binding {:action :visual_line
                                          :context :editor.normal}
                                :event {:action :visual_line
                                        :type :editing/action}
                                :id :editor.visual_line
                                :label "Editor: visual line"}
           :editor.word_end {:available editing.available?
                             :binding {:action :word_end
                                       :context :editor.normal}
                             :event {:action :word_end :type :editing/action}
                             :id :editor.word_end
                             :label "Editor: word end"}
           :editor.word_next {:available editing.available?
                              :binding {:action :word_next
                                        :context :editor.normal}
                              :event {:action :word_next :type :editing/action}
                              :id :editor.word_next
                              :label "Editor: word next"}
           :editor.word_previous {:available editing.available?
                                  :binding {:action :word_previous
                                            :context :editor.normal}
                                  :event {:action :word_previous
                                          :type :editing/action}
                                  :id :editor.word_previous
                                  :label "Editor: word previous"}
           :editor.yank {:available editing.available?
                         :binding {:action :yank :context :editor.normal}
                         :event {:action :yank :type :editing/action}
                         :id :editor.yank
                         :label "Editor: yank"}}
 :editing-actions {:normal editing.normal
                   :submit editing.submit
                   :insert editing.insert
                   :append editing.insert
                   :insert_start editing.insert
                   :append_end editing.insert
                   :open_below editing.insert
                   :open_above editing.insert
                   :visual editing.visual
                   :visual_line editing.visual
                   :undo editing.undo
                   :redo editing.undo
                   :paste editing.paste}
 :editing-motions {:left (fn [editor]
                           (editing.prev editor.text editor.cursor))
                   :right (fn [editor]
                            (editing.next-at editor.text editor.cursor))
                   :line_start (fn [editor]
                                 (editing.line-start editor.text editor.cursor))
                   :line_end (fn [editor]
                               (editing.line-end editor.text editor.cursor))
                   :down (fn [editor] (editing.vertical editor :down))
                   :up (fn [editor] (editing.vertical editor :up))
                   :first (fn [] 0)
                   :last (fn [editor] (length editor.text))
                   :word_next editing.word-next
                   :word_previous editing.word-previous
                   :word_end editing.word-end}
 :events {:editing/editing/action {:event :editing/action
                                   :handler editing.action
                                   :priority 71000}
          :editing/editing/interrupt {:event :editing/interrupt
                                      :handler editing.on-editing-interrupt
                                      :priority 71000}}
 :keybindings {:editor.normal/append {:action :append
                                      :context :editor.normal
                                      :default [:a]}
               :editor.normal/append_end {:action :append_end
                                          :context :editor.normal
                                          :default [:A]}
               :editor.normal/change {:action :change
                                      :context :editor.normal
                                      :default [:c]}
               :editor.normal/delete {:action :delete
                                      :context :editor.normal
                                      :default [:d]}
               :editor.normal/delete_character {:action :delete_character
                                                :context :editor.normal
                                                :default [:x]}
               :editor.normal/down {:action :down
                                    :context :editor.normal
                                    :default [:j :arrow_down]}
               :editor.normal/first {:action :first
                                     :context :editor.normal
                                     :default [:g]}
               :editor.normal/insert {:action :insert
                                      :context :editor.normal
                                      :default [:i]}
               :editor.normal/insert_start {:action :insert_start
                                            :context :editor.normal
                                            :default [:I]}
               :editor.normal/last {:action :last
                                    :context :editor.normal
                                    :default [:G]}
               :editor.normal/left {:action :left
                                    :context :editor.normal
                                    :default [:h :arrow_left]}
               :editor.normal/line_end {:action :line_end
                                        :context :editor.normal
                                        :default ["$"]}
               :editor.normal/line_start {:action :line_start
                                          :context :editor.normal
                                          :default [:0]}
               :editor.normal/normal {:action :normal
                                      :context :editor.normal
                                      :default [:escape]}
               :editor.normal/open_above {:action :open_above
                                          :context :editor.normal
                                          :default [:O]}
               :editor.normal/open_below {:action :open_below
                                          :context :editor.normal
                                          :default [:o]}
               :editor.normal/paste {:action :paste
                                     :context :editor.normal
                                     :default [:p]}
               :editor.normal/redo {:action :redo
                                    :context :editor.normal
                                    :default [:U]}
               :editor.normal/right {:action :right
                                     :context :editor.normal
                                     :default [:l :arrow_right]}
               :editor.normal/submit {:action :submit
                                      :context :editor.normal
                                      :default [:enter]}
               :editor.normal/undo {:action :undo
                                    :context :editor.normal
                                    :default [:u]}
               :editor.normal/up {:action :up
                                  :context :editor.normal
                                  :default [:k :arrow_up]}
               :editor.normal/visual {:action :visual
                                      :context :editor.normal
                                      :default [:v]}
               :editor.normal/visual_line {:action :visual_line
                                           :context :editor.normal
                                           :default [:V]}
               :editor.normal/word_end {:action :word_end
                                        :context :editor.normal
                                        :default [:e]}
               :editor.normal/word_next {:action :word_next
                                         :context :editor.normal
                                         :default [:w]}
               :editor.normal/word_previous {:action :word_previous
                                             :context :editor.normal
                                             :default [:b]}
               :editor.normal/yank {:action :yank
                                    :context :editor.normal
                                    :default [:y]}}
 :routes {:editing/input {:id :editing/input
                          :event :terminal/input
                          :priority 200
                          :context [:db/path]
                          :resolve (fn [db event cofx]
                                     (editing.route-input db event cofx
                                                          (editing.enabled? cofx.config)))}}
 :services {:editor.transition (fn [state previous editor reason interactive]
                                 (editing.editor-transition (editing.enabled? (misa.configuration))
                                                            state previous
                                                            editor reason
                                                            interactive))}
 :validators {:editing-motions editing.editing-motions
              :editing-actions editing.editing-actions}}
