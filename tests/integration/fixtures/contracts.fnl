(fn append [items item]
  (local result (icollect [_ value (ipairs (or items []))] value))
  (table.insert result item)
  result)

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
                                          (misa.patch tx {:db {:order (misa.replace
                                                                       (append tx.db.order :after))}}))
                                 :before (fn [tx]
                                           (misa.patch tx {:db {:order (misa.replace
                                                                        (append tx.db.order :before))}}))
                                 :id :trace}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db event cofx]
                                    (assert (= cofx.config.nested.value 7))
                                    {:patch {:order (misa.replace (append db.order (.. "first:" cofx.ordered)))}
                                     :fx [{:type :test/next}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    {:patch {:order (misa.replace (append db.order :second))}})})
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
