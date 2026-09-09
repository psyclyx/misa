(local definitions (require :tests.declarations))

(fn append [items item]
  (local result (icollect [_ value (ipairs (or items []))] value))
  (table.insert result item)
  result)

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :coeffects :id :derived :value (fn [] :derived)})
          (table.insert declarations
                        {:catalog :coeffects :id :ordered :value (fn [cofx] (.. cofx.derived ":ordered"))})
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {:patch {:order (misa.replace (append db.order :before))}})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db event cofx]
                                    (assert (= cofx.config.nested.value 7))
                                    {:patch {:order (misa.replace (append db.order (.. "first:" cofx.ordered)))}
                                     :fx [{:type :test/next}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {:patch {:order (misa.replace (append db.order :second))}})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {:patch {:order (misa.replace (append db.order :after))}})}})
          (table.insert declarations
                        {:catalog :effects :id :test/next :value (fn []
                                    {:event {:type :test/done} :type :dispatch})})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/done :handler (fn [db]
                                    {:patch {:order (misa.replace (append db.order :before))}})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/done :handler (fn [db event cofx]
                                    (assert (and (= cofx.config.nested.value 7)
                                                 (= (. cofx.argv 1) :original)))
                                    (local (ok err)
                                           (pcall (fn []
                                                    (misa._install {} {})
                                                    nil)))
                                    (assert (and (not ok) (err:find :sealed)))
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text (table.concat db.order
                                                                                 ",")}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          (table.insert declarations
                        {:catalog :views :id :main :value (fn [] {:cursor nil :lines {}})})
          nil
          (definitions.collect :tests.integration.fixtures.contracts declarations {}))
