(local selection (require :misa.selection))

{:actions {:selection.child {:available selection.selection-open?
                             :binding {:action :child :context :selection}
                             :event {:action :child :type :selection/action}
                             :id :selection.child
                             :label "Selection: child"}
           :selection.close {:available selection.selection-open?
                             :binding {:action :close :context :selection}
                             :event {:action :close :type :selection/action}
                             :id :selection.close
                             :label "Selection: close"}
           :selection.copy {:available selection.selection-open?
                            :binding {:action :copy :context :selection}
                            :event {:action :copy :type :selection/action}
                            :id :selection.copy
                            :label "Selection: copy"}
           :selection.down {:available selection.selection-open?
                            :binding {:action :down :context :selection}
                            :event {:action :down :type :selection/action}
                            :id :selection.down
                            :label "Selection: down"}
           :selection.extend_next {:available selection.selection-open?
                                   :binding {:action :extend_next
                                             :context :selection}
                                   :event {:action :extend_next
                                           :type :selection/action}
                                   :id :selection.extend_next
                                   :label "Selection: extend_next"}
           :selection.extend_previous {:available selection.selection-open?
                                       :binding {:action :extend_previous
                                                 :context :selection}
                                       :event {:action :extend_previous
                                               :type :selection/action}
                                       :id :selection.extend_previous
                                       :label "Selection: extend_previous"}
           :selection.first {:available selection.selection-open?
                             :binding {:action :first :context :selection}
                             :event {:action :first :type :selection/action}
                             :id :selection.first
                             :label "Selection: first"}
           :selection.last {:available selection.selection-open?
                            :binding {:action :last :context :selection}
                            :event {:action :last :type :selection/action}
                            :id :selection.last
                            :label "Selection: last"}
           :selection.left {:available selection.selection-open?
                            :binding {:action :left :context :selection}
                            :event {:action :left :type :selection/action}
                            :id :selection.left
                            :label "Selection: left"}
           :selection.next {:available selection.selection-open?
                            :binding {:action :next :context :selection}
                            :event {:action :next :type :selection/action}
                            :id :selection.next
                            :label "Selection: next"}
           :selection.open {:available (fn [db] (not db.picker))
                            :binding {:action :select_transcript
                                      :context :global}
                            :event {:type :selection/open}
                            :id :selection.open
                            :label "Navigate transcript"}
           :selection.parent {:available selection.selection-open?
                              :binding {:action :parent :context :selection}
                              :event {:action :parent :type :selection/action}
                              :id :selection.parent
                              :label "Selection: parent"}
           :selection.previous {:available selection.selection-open?
                                :binding {:action :previous
                                          :context :selection}
                                :event {:action :previous
                                        :type :selection/action}
                                :id :selection.previous
                                :label "Selection: previous"}
           :selection.right {:available selection.selection-open?
                             :binding {:action :right :context :selection}
                             :event {:action :right :type :selection/action}
                             :id :selection.right
                             :label "Selection: right"}
           :selection.up {:available selection.selection-open?
                          :binding {:action :up :context :selection}
                          :event {:action :up :type :selection/action}
                          :id :selection.up
                          :label "Selection: up"}
           :selection.visual {:available selection.selection-open?
                              :binding {:action :visual :context :selection}
                              :event {:action :visual :type :selection/action}
                              :id :selection.visual
                              :label "Selection: visual"}}
 :events {:selection/selection/action {:event :selection/action
                                       :handler selection.on-selection-action
                                       :priority 34000}
          :selection/selection/open {:event :selection/open
                                     :handler selection.on-selection-open
                                     :priority 34000}
          :selection/transcript/reset {:event :transcript/reset
                                       :handler (fn [_]
                                                  {:patch {:selection misa.delete}})
                                       :priority 34000}}
 :keybindings {:global/select_transcript {:action :select_transcript
                                          :context :global
                                          :default [:alt+s]}
               :selection/child {:action :child
                                 :context :selection
                                 :default [:J :shift+j :enter]}
               :selection/close {:action :close
                                 :context :selection
                                 :default [:escape :ctrl_c :q]}
               :selection/copy {:action :copy
                                :context :selection
                                :default [:y]}
               :selection/down {:action :down
                                :context :selection
                                :default [:j :arrow_down]}
               :selection/extend_next {:action :extend_next
                                       :context :selection
                                       :default [:shift+arrow_down]}
               :selection/extend_previous {:action :extend_previous
                                           :context :selection
                                           :default [:shift+arrow_up]}
               :selection/first {:action :first
                                 :context :selection
                                 :default [:g]}
               :selection/last {:action :last
                                :context :selection
                                :default [:G]}
               :selection/left {:action :left
                                :context :selection
                                :default [:h :arrow_left]}
               :selection/next {:action :next :context :selection :default []}
               :selection/parent {:action :parent
                                  :context :selection
                                  :default [:K :shift+k :backspace]}
               :selection/previous {:action :previous
                                    :context :selection
                                    :default []}
               :selection/right {:action :right
                                 :context :selection
                                 :default [:l :arrow_right]}
               :selection/up {:action :up
                              :context :selection
                              :default [:k :arrow_up]}
               :selection/visual {:action :visual
                                  :context :selection
                                  :default [:v]}}
 :routes {:selection/input {:id :selection/input
                            :event :terminal/input
                            :priority 500
                            :context [:db/path]
                            :resolve selection.route-terminal-input}}
 :selection-actions {:child selection.child
                     :close (fn [] {:state nil :close true})
                     :copy selection.copy
                     :down (fn [state db event cofx]
                             (selection.move-direction :down state db event
                                                       cofx))
                     :extend_next (fn [state]
                                    (selection.apply-motion :extend_next
                                                            (fn [frame]
                                                              (+ frame.index 1))
                                                            state))
                     :extend_previous (fn [state]
                                        (selection.apply-motion :extend_previous
                                                                (fn [frame]
                                                                  (- frame.index
                                                                     1))
                                                                state))
                     :first (fn [state]
                              (selection.apply-motion :first (fn [] 1) state))
                     :last (fn [state]
                             (selection.apply-motion :last
                                                     (fn [frame]
                                                       (length frame.nodes))
                                                     state))
                     :left (fn [state db event cofx]
                             (selection.move-direction :left state db event
                                                       cofx))
                     :next (fn [state]
                             (selection.apply-motion :next
                                                     (fn [frame]
                                                       (+ frame.index 1))
                                                     state))
                     :parent selection.parent
                     :previous (fn [state]
                                 (selection.apply-motion :previous
                                                         (fn [frame]
                                                           (- frame.index 1))
                                                         state))
                     :right (fn [state db event cofx]
                              (selection.move-direction :right state db event
                                                        cofx))
                     :up (fn [state db event cofx]
                           (selection.move-direction :up state db event cofx))
                     :visual selection.visual}
 :services {:selection.decorate selection.selection-decorate
            :selection.geometry selection.selection-geometry
            :selection.ranges selection.selection-ranges
            :selection.state selection.selection-state}
 :validators {:selection-actions selection.selection-actions
              :selection-sources selection.selection-sources}
 :view-layers {:selection {:handler selection.on-selection}}}
