{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/cofx
                         :name :policy
                         :handler (fn [] :derived)})
          (table.insert setup-fx
                        {:type :register/cofx
                         :name :ordered
                         :handler (fn [cofx] (.. cofx.policy ":ordered"))})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:after (fn [tx]
                                          (tset tx.db.order
                                                (+ (length tx.db.order) 1)
                                                :after)
                                          tx)
                                 :before (fn [tx]
                                           (set tx.db.order (or tx.db.order {}))
                                           (tset tx.db.order
                                                 (+ (length tx.db.order) 1)
                                                 :before)
                                           tx)
                                 :id :trace}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db event cofx]
                                    (assert (= cofx.config.nested.value 7))
                                    (tset db.order (+ (length db.order) 1)
                                          (.. "first:" cofx.ordered))
                                    {: db :fx [{:type :test/next}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (tset db.order (+ (length db.order) 1)
                                          :second)
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :test/next
                         :handler (fn []
                                    {:event {:type :test/done} :type :dispatch})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/done
                         :handler (fn [db event cofx]
                                    (assert (and (= cofx.config.nested.value 7)
                                                 (= (. cofx.argv 1) :original)))
                                    (local (ok err)
                                           (pcall (fn []
                                                    (misa._setup_effects {:fx [{:type :register/event
                                                                                :name :late
                                                                                :handler (fn []
                                                                                           nil)}]})
                                                    nil)))
                                    (assert (and (not ok) (err:find :sealed)))
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text (table.concat db.order
                                                                                 ",")}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          (table.insert setup-fx
                        {:type :register/view
                         :handler (fn [] {:cursor nil :lines {}})})
          nil
          {:fx setup-fx})}
