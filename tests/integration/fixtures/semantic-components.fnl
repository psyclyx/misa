{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/theme
                         :id :test
                         :value {:palette {:text :default}
                                 :styles {:plain {:foreground :text}}}})
          (assert (not (pcall (fn []
                                (misa._setup_effects {:fx [{:type :register/theme
                                                            :id :bad
                                                            :value {:palette {:oops :orange}
                                                                    :styles {}}}]})
                                nil)))
                  "invalid palette color was accepted")
          (assert (not (pcall (fn []
                                (misa._setup_effects {:fx [{:type :register/theme
                                                            :id :foundationless
                                                            :value {:palette {}
                                                                    :styles {:label {:dim true}}}}]})
                                nil)))
                  "foundationless theme was accepted")
          (table.insert setup-fx
                        {:type :register/animation
                         :id :pulse
                         :value {:frames [:one :two]}})
          (table.insert setup-fx
                        {:type :register/component
                         :id :test.first
                         :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text :first}]}]})}})
          (table.insert setup-fx
                        {:type :register/component
                         :id :test.second
                         :value {:render (fn []
                                           {:lines [{:spans [{:style :plain
                                                              :text :swapped}]}]})}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (assert (= (. (misa.render_component db
                                                                         :test.role
                                                                         {})
                                                  :lines 1 :spans 1 :text)
                                               :first))
                                    (assert (and (= (. (misa.theme_style db
                                                                         :syntax.escape)
                                                       :bold)
                                                    true)
                                                 (= (. (misa.theme_style db
                                                                         :dialog.hint)
                                                       :dim)
                                                    true))
                                            "complete standard theme fallbacks were not composed")
                                    (assert (= (misa.animation_frame db nil 1)
                                               :two))
                                    {:fx [{:event {:implementation :test.second
                                                   :role :test.role
                                                   :type :components/swap}
                                           :type :dispatch}
                                          {:event {:type :test/render}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/render
                         :handler (fn [db]
                                    {:fx [{:lines (. (misa.render_component db
                                                                            :test.role
                                                                            {})
                                                     :lines)
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
