(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :themes :id :test :value {:palette {:text :default}
                                 :styles {:plain {:foreground :text}}}})


          (table.insert declarations
                        {:catalog :animations :id :pulse :value {:frames [:one :two]}})
          (table.insert declarations
                        {:catalog :components :id :test.first :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text :first}]}]})}})
          (table.insert declarations
                        {:catalog :components :id :test.second :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text :swapped}]}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    (assert (= (. (misa.components.render db
                                                                         :test.role
                                                                         {})
                                                  :lines 1 :spans 1 :text)
                                               :first))
                                    (assert (and (= (. (misa.themes.style db
                                                                         :syntax.escape)
                                                       :bold)
                                                    true)
                                                 (= (. (misa.themes.style db
                                                                         :dialog.hint)
                                                       :dim)
                                                    true))
                                            "complete standard theme fallbacks were not composed")
                                    (assert (= (misa.animations.frame db nil 1)
                                               :two))
                                    {:fx [{:event {:implementation :test.second
                                                   :role :test.role
                                                   :type :components/swap}
                                           :type :dispatch}
                                          {:event {:type :test/render}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/render :handler (fn [db]
                                    {:fx [{:lines (. (misa.components.render db
                                                                            :test.role
                                                                            {})
                                                     :lines)
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions.build :tests.integration.fixtures.semantic-components declarations {}))
