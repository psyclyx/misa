(local history (require :misa.editor.history))

{:actions {:history.next {:available history.available?
                          :binding {:action :next :context :history}
                          :event {:type :history/next}
                          :id :history.next
                          :label "Next input"}
           :history.previous {:available history.available?
                              :binding {:action :previous :context :history}
                              :event {:type :history/previous}
                              :id :history.previous
                              :label "Previous input"}
           :history.search {:available history.available?
                            :binding {:action :search :context :history}
                            :event {:type :history/search}
                            :id :history.search
                            :label "Search input history"}}
 :events {"history/agent/submitted" {:event :agent/submitted
                                     :handler (fn [db event cofx]
                                                (history.on-agent-submitted cofx.config
                                                                            db
                                                                            event))
                                     :priority 70000}
          "history/app/start" {:event :app/start
                               :handler (fn [db event cofx]
                                          (history.on-app-start cofx.config db
                                                                event))
                               :priority 70000}
          "history/history/loaded" {:event :history/loaded
                                    :handler (fn [db event cofx]
                                               (history.on-history-loaded cofx.config
                                                                          db
                                                                          event))
                                    :priority 70000}
          "history/history/next" {:event :history/next
                                  :handler (fn [db]
                                             (history.navigate db (- 1)))
                                  :priority 70000}
          "history/history/previous" {:event :history/previous
                                      :handler (fn [db]
                                                 (history.navigate db 1))
                                      :priority 70000}
          "history/history/search" {:event :history/search
                                    :handler history.on-history-search
                                    :priority 70000}
          "history/history/selected" {:event :history/selected
                                      :handler history.on-history-selected
                                      :priority 70000}}
 :keybindings {:history/next {:action :next
                              :context :history
                              :default [:ctrl_n :alt+n]}
               :history/previous {:action :previous
                                  :context :history
                                  :default [:ctrl_p :alt+p]}
               :history/search {:action :search
                                :context :history
                                :default [:ctrl_r]}}
 :routes {:history/input {:id :history/input
                          :event :terminal/input
                          :priority 300
                          :context [:db/path]
                          :resolve history.route-terminal-input}}}
