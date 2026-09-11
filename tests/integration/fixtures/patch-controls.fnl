(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  ;; Seed an array, then rewrite and grow it with the patch controls.
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn [_ _]
                                    {:patch {:example {:items [{:id 1} {:id 2}]}}
                                     :fx [{:event {:type :test/indexed}
                                           :type :dispatch}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :test/indexed
                         :handler (fn [db _]
                                    (let [items db.example.items
                                          written (misa.patch db
                                                              {:example {:items (misa.at 1
                                                                                         {:id 9})}})
                                          grown (misa.patch written
                                                            {:example {:items (misa.append {:id 3})}})
                                          next (assert grown.example.items)]
                                      (assert (= (length items) 2)
                                              "the start state is wrong")
                                      (assert (= (length next) 3)
                                              "the patch controls did not grow the array")
                                      (assert (= (. next 1 :id) 9)
                                              "misa.at did not replace an element")
                                      (assert (= (. next 2) (. items 2))
                                              "misa.at copied an unchanged element")
                                      (assert (= (. next 3 :id) 3)
                                              "misa.append did not append")
                                      (assert (= (. items 1 :id) 1)
                                              "a patch control mutated its input")
                                      (assert (not (pcall misa.patch db
                                                          {:example {:items (misa.at 9
                                                                                     1)}}))
                                              "an out-of-range index was accepted")
                                      (assert (not (pcall misa.patch db
                                                          {:example {:items (misa.replace [(misa.append 1)])}}))
                                              "an append was accepted inside replacement data")
                                      {:patch {:example {:items next}}
                                       :fx [{:lines [{:spans [{:text :patched}]}]
                                             :type :view/commit}
                                            {:type :app/quit}]}))}})
  nil
  (definitions.collect :tests.integration.fixtures.patch-controls
    declarations
    {}))
