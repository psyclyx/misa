(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (var received nil)
          (table.insert declarations
                        {:catalog :components :id :test.immutable :value {:render (fn [model]
                                           (set received model)
                                           {:lines [{:spans [{:style :plain
                                                              :text "plain then **bold** and é界"}]}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    (local failures
                                           [(pcall (fn []
                                                     (misa._install {} {})
                                                     nil))
                                            (pcall (fn []
                                                     (misa._install {} {})
                                                     nil))
                                            (pcall (fn []
                                                     (misa._install {} {})
                                                     nil))])
                                    (assert (and (and (not (. failures 1))
                                                      (not (. failures 2)))
                                                 (not (. failures 3)))
                                            "semantic registries did not seal at app/start")
                                    (local render-db (misa.patch db {:marker :original}))
                                    (local model {:text :original})
                                    (misa.components.render render-db :test.role model
                                                           {})
                                    (assert (= received model)
                                            "component boundary copied immutable input")
                                    (assert (and (= render-db.marker :original)
                                                 (= model.text :original))
                                            "component projection mutated canonical input")
                                    (local clipped
                                           (misa.ui.bound-frame [{:spans [{:text :a}
                                                                          {:link "https://one"
                                                                           :style {:bold true}
                                                                           :text "👩‍"}
                                                                          {:link "https://two"
                                                                           :style {:underline true}
                                                                           :text "💻z"}]}]
                                                                3
                                                                {:byte 12
                                                                 :row 1}))
                                    (assert (and (and (and (and (= (length (. clipped.lines
                                                                              1
                                                                              :spans))
                                                                   3)
                                                                (= (. clipped.lines
                                                                      1 :spans 2
                                                                      :text)
                                                                   "👩‍"))
                                                           (= (. clipped.lines
                                                                 1 :spans 2
                                                                 :link)
                                                              "https://one"))
                                                      (= (. clipped.lines 1
                                                            :spans 3 :text)
                                                         "💻"))
                                                 (= (. clipped.lines 1 :spans 3
                                                       :link)
                                                    "https://two"))
                                            "final clipping split a cross-span grapheme or erased styling/link boundaries")
                                    (assert (= clipped.cursor.byte 12)
                                            "final clipping produced an invalid grapheme cursor")
                                    (local omitted
                                           (misa.ui.bound-frame [{:spans [{:text :a}
                                                                          {:text "👩‍"}
                                                                          {:text "💻z"}]}]
                                                                2
                                                                {:byte 5
                                                                 :row 1}))
                                    (assert (and (and (= (length (. omitted.lines
                                                                    1 :spans))
                                                         1)
                                                      (= (. omitted.lines 1
                                                            :spans 1 :text)
                                                         :a))
                                                 (= omitted.cursor.byte 1))
                                            "clipped grapheme fragment or split cursor survived")
                                    {:fx [{:event {:status :working
                                                   :type :agent/status}
                                           :type :dispatch}
                                          {:event {:kind :text
                                                   :text :ignored
                                                   :type :terminal/input}
                                           :type :dispatch}
                                          {:event {:type :test/busy-done}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/busy-done :handler (fn [db event]
                                    (assert (= db.editor.text :ignored)
                                            "busy input was discarded")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text "review regressions"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.review-regressions declarations {}))
