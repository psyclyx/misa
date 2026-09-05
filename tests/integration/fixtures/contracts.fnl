{:setup (fn []
          (misa.reg_cofx :policy (fn [] :derived))
          (misa.reg_cofx :ordered (fn [cofx] (.. cofx.policy ":ordered")))
          (misa.reg_interceptor {:after (fn [tx]
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
                                 :id :trace})
          (misa.reg_event :app/start
                          (fn [db event cofx]
                            (assert (= cofx.config.nested.value 7))
                            (tset db.order (+ (length db.order) 1)
                                  (.. "first:" cofx.ordered))
                            {: db :fx [{:type :test/next}]}))
          (misa.reg_event :app/start
                          (fn [db]
                            (tset db.order (+ (length db.order) 1) :second)
                            {: db}))
          (misa.reg_fx :test/next
                       (fn [] {:event {:type :test/done} :type :dispatch}))
          (misa.reg_event :test/done
                          (fn [db event cofx]
                            (assert (and (= cofx.config.nested.value 7)
                                         (= (. cofx.argv 1) :original)))
                            (local (ok err)
                                   (pcall (fn []
                                            (misa.reg_event :late (fn [] nil))
                                            nil)))
                            (assert (and (not ok) (err:find :sealed)))
                            {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                     :text (table.concat db.order
                                                                         ",")}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          (misa.reg_view (fn [] {:cursor nil :lines {}}))
          nil)}

