{:setup (fn []
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:event {:cancellable true
                                           :completion :dialog/done
                                           :correlation :a
                                           :id :one
                                           :kind :progress
                                           :message :waiting
                                           :title :Work
                                           :type :dialog/open}
                                   :type :dispatch}]}))
          (misa.reg_event :dialog/done
                          (fn [db event]
                            (if (= event.correlation :a)
                                (do
                                  (assert (and event.cancelled
                                               (= db.dialog nil))
                                          "cancel did not close its dialog")
                                  {: db
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
                                                         (= event.value :code))
                                                    (= event.action :submit))
                                               (= db.dialog nil))
                                          "submit lifecycle failed")
                                  {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                           :text :dialogs}]}]
                                         :type :view/commit}
                                        {:type :app/quit}]}))))
          (misa.reg_event :dialog/opened-test (fn [] nil))
          (misa.reg_event :dialog/begin-cancel
                          (fn []
                            {:fx [{:event {:kind :escape :type :dialog/input}
                                   :type :dispatch}]}))
          (misa.reg_interceptor {:after (fn [tx]
                                          (when (and (= tx.event.type
                                                        :dialog/open)
                                                     (= tx.event.id :one))
                                            (tset tx.fx (+ (length tx.fx) 1)
                                                  {:event {:correlation :stale
                                                           :id :one
                                                           :message :bad
                                                           :type :dialog/update}
                                                   :type :dispatch})
                                            (tset tx.fx (+ (length tx.fx) 1)
                                                  {:event {:type :dialog/begin-cancel}
                                                   :type :dispatch}))
                                          tx)
                                 :id :dialog-test-cancel})
          nil)}

