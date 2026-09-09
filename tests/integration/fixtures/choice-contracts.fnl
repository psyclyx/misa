(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    (var ordered
                                           (misa.choices.session {:items [{:value :vendor/one}]
                                                                 :purpose :models
                                                                 :title :Models}
                                                                db))
                                    (assert (and (= (. ordered.view_ids 1)
                                                    :frecency)
                                                 (= (. ordered.view_ids 2) :all))
                                            "purpose view order was ignored")
                                    (each [_ item (ipairs (misa.choices.registered-views ordered))]
                                      (assert (not= item.value :slash-prefix)
                                              "removed tree view is still registered"))
                                    (local flat
                                           (misa.choices.session {:items [{:description :vendor/model
                                                                          :label :vendor/model
                                                                          :search ["Hidden Label"]
                                                                          :value :vendor/model}]
                                                                 :purpose :generic
                                                                 :query :hidden
                                                                 :title :Flat
                                                                 :views [:all]}
                                                                db))
                                    (local flat-rows
                                           (. (misa.choices.rows flat db) 1
                                              :rows))
                                    (assert (and (= (length flat-rows) 1)
                                                 (= (. flat-rows 1 :description)
                                                    nil))
                                            "flat rows repeated redundant model text or lost hidden search")
                                    (local history
                                           {:preferences {:scopes {:models {:one {:last 10
                                                                                  :uses 1}
                                                                            :two {:last 20
                                                                                  :uses 2}}}}})
                                    (var browse
                                           (misa.choices.session {:items [{:value :one}
                                                                         {:value :two}
                                                                         {:value :three}]
                                                                 :preference_scope :models
                                                                 :title :Models
                                                                 :views [:browse]}
                                                                history))
                                    (local browse-rows
                                           (. (misa.choices.rows browse history)
                                              1 :rows))
                                    (assert (and (and (= (. browse-rows 1
                                                            :value)
                                                         :two)
                                                      (= (. browse-rows 1
                                                            :section)
                                                         :Recent))
                                                 (= (. browse-rows 3 :section)
                                                    :All))
                                            "browse lost recent ordering or all section")
                                    (set browse.query :t)
                                    (set browse (misa.choices.refresh browse history))
                                    (each [_ row (ipairs (. (misa.choices.rows browse
                                                                              history)
                                                            1 :rows))]
                                      (assert (= row.section nil)
                                              "search kept recent grouping"))
                                    (local panels
                                           (misa.choices.session {:purpose :generic
                                                                 :title :Panels
                                                                 :view_definitions [{:id :one
                                                                                     :items [{:value :first}]}
                                                                                    {:id :two
                                                                                     :items [{:value :second}]}]}
                                                                db))
                                    (assert (= (misa.choices.positional panels
                                                                       :option_2_1
                                                                       1 5)
                                               nil)
                                            "hidden positional bank activated")
                                    (assert (= (misa.choices.positional panels
                                                                       :option_1_2
                                                                       1 1)
                                               nil)
                                            "hidden positional slot activated")
                                    (assert (= (. (misa.choices.positional panels
                                                                          :option_2_1
                                                                          2 5)
                                                  :value)
                                               :second)
                                            "visible positional bank did not activate")
                                    (local narrow
                                           (misa.choices.picker-layout panels db
                                                                      {:columns 57
                                                                       :lines 8}))
                                    (assert (and (= narrow.panel_count 1)
                                                 (= narrow.targets.option_2_1
                                                    nil))
                                            "nonrendered picker panel retained an active hotkey")
                                    (local wrapped
                                           (misa.choices.session {:items [{:label (string.rep "long "
                                                                                             20)
                                                                          :value :one}
                                                                         {:value :two}]
                                                                 :title :Wrapped
                                                                 :views [:all]}
                                                                db))
                                    (local visible
                                           (misa.choices.picker-layout wrapped
                                                                      db
                                                                      {:columns 30
                                                                       :lines 6}))
                                    (assert (and (and (= (length (. visible.columns
                                                                    1 :rows))
                                                         1)
                                                      (= visible.targets.option_1_1.value
                                                         :one))
                                                 (<= (length (. visible.columns
                                                                1 :lines))
                                                     (- visible.panel_height 1)))
                                            "oversized choice lost its visible target or escaped the panel budget")
                                    (var selected
                                           (misa.choices.session {:items [{:label (string.rep "long "
                                                                                             8)
                                                                          :value :one}
                                                                         {:label (string.rep "selected "
                                                                                             5)
                                                                          :value :two}
                                                                         {:value :three}]
                                                                 :selected :two
                                                                 :title :Selected
                                                                 :views [:all]}
                                                                db))
                                    (set selected (. (misa.choices.input selected {:action :next}
                                                       db) :session))
                                    (local measured
                                           (misa.choices.picker-layout selected
                                                                      db
                                                                      {:columns 40
                                                                       :lines 8}))
                                    (assert (and (= measured.x 2)
                                                 (= measured.width 36))
                                            "picker was centered or width constrained")
                                    (var found false)
                                    (each [_ row (ipairs (. measured.columns 1
                                                            :rows))]
                                      (when (= row.value :two) (set found true)))
                                    (assert (and found
                                                 (= (length (. measured.columns 1 :rows)) 3)
                                                 (not (. measured.columns 1 :overflow)))
                                            (.. "fitting choices should retain focus without overflow: "
                                                (length (. measured.columns 1
                                                           :rows))
                                                " " (tostring found) " "
                                                (tostring (. measured.columns 1
                                                             :overflow))))
                                    (local styled
                                           (misa.choices.row-lines {:hotkey :alt+1
                                                                   :label (string.rep "selected "
                                                                                      5)
                                                                   :marker " "
                                                                   :selected true}
                                                                  20))
                                    (assert (> (length styled) 1))
                                    (each [_ line (ipairs styled)]
                                      (var width 0)
                                      (each [_ span (ipairs line.spans)]
                                        (assert (= span.style
                                                   :choice.row.selected)
                                                "wrapped selected span lost its style")
                                        (set width
                                             (+ width
                                                (misa.layout.width span.text))))
                                      (assert (= width 20)
                                              "selected row did not fill its width"))
                                    (local no-budget
                                           (misa.choices.picker-layout flat db
                                                                      {:available_lines 2
                                                                       :columns 30}))
                                    (assert (and (= (length (. no-budget.columns
                                                               1 :rows))
                                                    0)
                                                 (= (next no-budget.targets)
                                                    nil))
                                            "zero panel budget exposed a row or target")
                                    (local prefixed
                                           (misa.choices.session {:input_prefix "/model "
                                                                 :items [{:value :vendor/model}]
                                                                 :query :vendor/
                                                                 :title :Arguments
                                                                 :views [:all]}
                                                                db))
                                    (local prefixed-layout
                                           (misa.choices.picker-layout prefixed
                                                                      db
                                                                      {:columns 40
                                                                       :lines 8}))
                                    (assert (and (= prefixed-layout.input.text
                                                    "/model vendor/")
                                                 (= prefixed-layout.input.cursor
                                                    (length prefixed-layout.input.text)))
                                            "narrowed picker omitted its canonical command prefix")
                                    (var parent
                                           (misa.choices.session {:purpose :generic
                                                                 :title :Parent
                                                                 :view_definitions [{:id :parent
                                                                                     :items [{:value :parent}]}]}
                                                                db))
                                    (assert (and (and (= parent.selected nil)
                                                      (= parent.preference_scope
                                                         nil))
                                                 (not= parent.custom_views nil)))
                                    (set parent (. (misa.choices.accept parent
                                                        {:narrow {:items [{:value :child}]
                                                                  :preference_scope :child-scope
                                                                  :purpose :generic
                                                                  :selected :child
                                                                  :title :Child}}
                                                        db) :session))
                                    (assert (and (and (= parent.selected :child)
                                                      (= parent.preference_scope
                                                         :child-scope))
                                                 (= parent.custom_views nil))
                                            "parent-only narrowing state leaked into child")
                                    (set parent (. (misa.choices.input parent {:action :cancel}
                                                       db) :session))
                                    (assert (and (and (and (= parent.selected
                                                              nil)
                                                           (= parent.preference_scope
                                                              nil))
                                                      (not= parent.custom_views
                                                            nil))
                                                 (= (. parent.view_ids 1)
                                                    :parent))
                                            "narrow frame did not restore absent and custom state exactly")
                                    (assert (= (misa.choices.action {:key :alt+x
                                                                    :kind :key})
                                               :open_overlay)
                                            "configured shared action was not resolved")
                                    (assert (= (misa.choices.hint :option_1_1)
                                               :alt+z)
                                            "configured shared positional hint was not resolved")
                                    (set ordered (misa.choices.replace-view ordered :browse
                                                              db))
                                    (local fresh
                                           (misa.choices.session {:items [{:value :vendor/one}]
                                                                 :purpose :models
                                                                 :title :Models}
                                                                db))
                                    (assert (= (. fresh.view_ids 1) :frecency)
                                            "view replacement leaked beyond its session")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text "choice contracts"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions :tests.integration.fixtures.choice-contracts declarations {}))
