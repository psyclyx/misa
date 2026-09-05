{:setup (fn []
          (misa.reg_theme :test
                          {:palette {:text :default}
                           :styles {:plain {:foreground :text}}})
          (assert (not (pcall (fn []
                                (misa.reg_theme :bad
                                                {:palette {:oops :orange}
                                                 :styles {}})
                                nil)))
                  "invalid palette color was accepted")
          (assert (not (pcall (fn []
                                (misa.reg_theme :foundationless
                                                {:palette {}
                                                 :styles {:label {:dim true}}})
                                nil)))
                  "foundationless theme was accepted")
          (misa.reg_animation :pulse {:frames [:one :two]})
          (misa.reg_component :test.first
                              {:render (fn []
                                         {:lines [{:spans [{:style :plain
                                                            :text :first}]}]})})
          (misa.reg_component :test.second
                              {:render (fn []
                                         {:lines [{:spans [{:style :plain
                                                            :text :swapped}]}]})})
          (misa.reg_event :app/start
                          (fn [db]
                            (assert (= (. (misa.render_component db :test.role
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
                            (assert (= (misa.animation_frame db nil 1) :two))
                            {:fx [{:event {:implementation :test.second
                                           :role :test.role
                                           :type :components/swap}
                                   :type :dispatch}
                                  {:event {:type :test/render} :type :dispatch}]}))
          (misa.reg_event :test/render
                          (fn [db]
                            {:fx [{:lines (. (misa.render_component db
                                                                    :test.role
                                                                    {})
                                             :lines)
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

