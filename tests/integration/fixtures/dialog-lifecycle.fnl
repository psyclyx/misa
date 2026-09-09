(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    {:fx [{:event {:cancellable true
                                                   :completion :dialog/done
                                                   :correlation :a
                                                   :id :one
                                                   :kind :progress
                                                   :message :waiting
                                                   :title :Work
                                                   :type :dialog/open}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :dialog/done :handler (fn [db event]
                                    (if (= event.correlation :a)
                                        (do
                                          (assert (and event.cancelled
                                                       (= db.dialog nil))
                                                  "cancel did not close its dialog")
                                          {
                                           :fx [{:event {:actions [{:id :submit
                                                                    :label :submit}]
                                                         :completion :dialog/done
                                                         :correlation :b
                                                         :id :two
                                                         :input true
                                                         :kind :modal
                                                         :message :paste
                                                         :title :Input
                                                         :type :dialog/open}
                                                 :type :dispatch}
                                                {:event {:kind :text
                                                         :text :code
                                                         :type :dialog/input}
                                                 :type :dispatch}
                                                {:event {:kind :enter
                                                         :type :dialog/input}
                                                 :type :dispatch}]})
                                        (do
                                          (assert (and (and (and (not event.cancelled)
                                                                 (= event.value
                                                                    :code))
                                                            (= event.action
                                                               :submit))
                                                       (= db.dialog nil))
                                                  "submit lifecycle failed")
                                          {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                                   :text :dialogs}]}]
                                                 :type :view/commit}
                                                {:type :app/quit}]})))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :dialog/opened-test :handler (fn [] nil)}})
          (table.insert declarations
                        {:catalog :events  :value {:event :dialog/begin-cancel :handler (fn []
                                    {:fx [{:event {:kind :escape
                                                   :type :dialog/input}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :dialog/open :handler (fn [_ event]
                                    (when (= event.id :one)
                                      {:fx [{:type :dispatch
                                             :event {:type :dialog/update :id :one
                                                     :correlation :stale :message :bad}}
                                            {:type :dispatch
                                             :event {:type :dialog/begin-cancel}}]}))}})
          nil
          (definitions :tests.integration.fixtures.dialog-lifecycle declarations {}))
