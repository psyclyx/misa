(local choices (require :misa.choices))

(let [catalogs {:actions {:choices.accept {:available (fn [db]
                                                        (choices.action-available? :accept
                                                                                   db))
                                           :binding {:action :accept
                                                     :context :choices}
                                           :event {:action :accept
                                                   :type :choices/dispatch}
                                           :id :choices.accept
                                           :label "Accept choice"}
                          :choices.cancel {:available (fn [db]
                                                        (choices.action-available? :cancel
                                                                                   db))
                                           :binding {:action :cancel
                                                     :context :choices}
                                           :event {:action :cancel
                                                   :type :choices/dispatch}
                                           :id :choices.cancel
                                           :label "Cancel choices / back"}
                          :choices.complete {:available (fn [db]
                                                          (choices.action-available? :complete
                                                                                     db))
                                             :binding {:action :complete
                                                       :context :choices}
                                             :event {:action :complete
                                                     :type :choices/dispatch}
                                             :id :choices.complete
                                             :label "Complete to branch"}
                          :choices.cycle {:available (fn [db]
                                                       (choices.action-available? :cycle
                                                                                  db))
                                          :binding {:action :cycle
                                                    :context :choices}
                                          :event {:action :cycle
                                                  :type :choices/dispatch}
                                          :id :choices.cycle
                                          :label "Next choice view"}
                          :choices.cycle_previous {:available (fn [db]
                                                                (choices.action-available? :cycle_previous
                                                                                           db))
                                                   :binding {:action :cycle_previous
                                                             :context :choices}
                                                   :event {:action :cycle_previous
                                                           :type :choices/dispatch}
                                                   :id :choices.cycle_previous
                                                   :label "Previous choice view"}
                          :choices.favorite {:available (fn [db]
                                                          (choices.action-available? :favorite
                                                                                     db))
                                             :binding {:action :favorite
                                                       :context :choices}
                                             :event {:action :favorite
                                                     :type :choices/dispatch}
                                             :id :choices.favorite
                                             :label "Toggle favorite choice"}
                          :choices.next {:available (fn [db]
                                                      (choices.action-available? :next
                                                                                 db))
                                         :binding {:action :next
                                                   :context :choices}
                                         :event {:action :next
                                                 :type :choices/dispatch}
                                         :id :choices.next
                                         :label "Next choice"}
                          :choices.open_overlay {:available (fn [db]
                                                              (choices.action-available? :open_overlay
                                                                                         db))
                                                 :binding {:action :open_overlay
                                                           :context :choices}
                                                 :event {:action :open_overlay
                                                         :type :choices/dispatch}
                                                 :id :choices.open_overlay
                                                 :label "Expand inline choices"}
                          :choices.previous {:available (fn [db]
                                                          (choices.action-available? :previous
                                                                                     db))
                                             :binding {:action :previous
                                                       :context :choices}
                                             :event {:action :previous
                                                     :type :choices/dispatch}
                                             :id :choices.previous
                                             :label "Previous choice"}
                          :choices.replace_view {:available (fn [db]
                                                              (choices.action-available? :replace_view
                                                                                         db))
                                                 :binding {:action :replace_view
                                                           :context :choices}
                                                 :event {:action :replace_view
                                                         :type :choices/dispatch}
                                                 :id :choices.replace_view
                                                 :label "Replace choice view"}}
                :choice-inputs {:text choices.insert-text
                                :backspace choices.backspace
                                :previous (fn [session _ db]
                                            (choices.move session (- 1) db))
                                :next (fn [session _ db]
                                        (choices.move session 1 db))
                                :cycle (fn [session _ db]
                                         (choices.cycle session 1 db))
                                :cycle_previous (fn [session _ db]
                                                  (choices.cycle session (- 1)
                                                                 db))
                                :replace_view (fn [session]
                                                {: session
                                                 :consumed true
                                                 :replace_view true})
                                :open_overlay (fn [session]
                                                {: session
                                                 :consumed true
                                                 :open_overlay true})
                                :favorite choices.favorite
                                :cancel choices.cancel
                                :accept choices.accept-focused}
                :choice-views {:all {:title :All}
                               :browse {:project choices.browse :title :Browse}
                               :favorites {:include choices.favorite?
                                           :title :Favorites}
                               :frecency {:include choices.used?
                                          :order choices.recent-before?
                                          :title :Recent}}
                :events {:choices/choices/dispatch {:event :choices/dispatch
                                                    :handler choices.on-choices-dispatch
                                                    :priority 21000}
                         :choices/choices/ignored {:event :choices/ignored
                                                   :handler (fn []
                                                              {:fx [{:type :terminal/read}]})
                                                   :priority 21000}}
                :keybindings {:choices/accept {:action :accept
                                               :context :choices
                                               :default [:enter]}
                              :choices/cancel {:action :cancel
                                               :context :choices
                                               :default [:escape
                                                         :ctrl_c
                                                         :ctrl_d
                                                         :eof]}
                              :choices/complete {:action :complete
                                                 :context :choices
                                                 :default [:tab]}
                              :choices/cycle {:action :cycle
                                              :context :choices
                                              :default [:arrow_right]}
                              :choices/cycle_previous {:action :cycle_previous
                                                       :context :choices
                                                       :default [:arrow_left
                                                                 :alt+p]}
                              :choices/favorite {:action :favorite
                                                 :context :choices
                                                 :default [:alt+v]}
                              :choices/next {:action :next
                                             :context :choices
                                             :default [:arrow_down]}
                              :choices/open_overlay {:action :open_overlay
                                                     :context :choices
                                                     :default []}
                              :choices/previous {:action :previous
                                                 :context :choices
                                                 :default [:arrow_up]}
                              :choices/replace_view {:action :replace_view
                                                     :context :choices
                                                     :default [:alt+/]}}
                :routes {:choices/sequence {:id :choices/sequence
                                            :event :terminal/input
                                            :priority 950
                                            :context [:db/path]
                                            :resolve choices.route-terminal-input}
                         :choices/visible-action {:id :choices/visible-action
                                                  :event :ui/action
                                                  :priority 800
                                                  :context [:db/path]
                                                  :resolve choices.route-ui-action}}
                :services {: choices.accept
                           : choices.action
                           :choices.first-index choices.first
                           : choices.hint
                           :choices.hotkeys choices.choices-hotkeys
                           : choices.input
                           :choices.needs-targets? choices.needs-targets
                           :choices.pending choices.choices-pending
                           :choices.positional choices.choices-positional
                           :choices.projected-rows choices.choices-projected-rows
                           : choices.refresh
                           :choices.registered-views choices.choices-registered-views
                           :choices.replace-view choices.choices-replace-view
                           :choices.rows choices.choices-rows
                           :choices.session (fn [spec db]
                                              (choices.new (choices.options (misa.configuration))
                                                           spec db))
                           :choices.set-items choices.choices-set-items
                           :choices.source choices.choices-source}
                :subscriptions {:choices/builtin-items {:id :choices/builtin-items
                                                        :inputs [[:db/path
                                                                  :items]
                                                                 [:db/path
                                                                  :query]
                                                                 [:db/path
                                                                  :scope]
                                                                 [:db/path
                                                                  :preferences]
                                                                 [:db/path
                                                                  :prior]
                                                                 [:db/path
                                                                  :view]
                                                                 [:db/path
                                                                  :config]]
                                                        :compute choices.compute-choices-builtin-items}}
                :validators {:choice-inputs choices.choice-inputs
                             :choice-views choices.choice-views
                             :choice-sources choices.choice-sources}}]
  (each [bank keys (ipairs choices.banks)]
    (for [slot 1 9]
      (let [name (.. :option_ bank "_" slot)]
        (tset catalogs.keybindings (.. :choices/ name)
              {:action name
               :context :choices
               :default (if (and (= bank 1) (. keys slot))
                            [(.. :alt+ (. keys slot))]
                            [])})
        (tset catalogs.actions (.. :choices. name)
              {:available choices.session-open?
               :binding {:action name :context :choices}
               :event {:action name :type :choices/dispatch}
               :id (.. :choices. name)
               :label (.. "Choose visible item " bank ":" slot)}))))
  catalogs)
