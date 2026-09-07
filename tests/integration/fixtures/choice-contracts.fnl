{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (var ordered
                                           (misa.choice_session {:items [{:value :vendor/one}]
                                                                 :purpose :models
                                                                 :title :Models}
                                                                db))
                                    (assert (and (= (. ordered.view_ids 1)
                                                    :frecency)
                                                 (= (. ordered.view_ids 2) :all))
                                            "purpose view order was ignored")
                                    (each [_ item (ipairs (misa.choice_registered_views ordered))]
                                      (assert (not= item.value :slash-prefix)
                                              "removed tree view is still registered"))
                                    (local flat
                                           (misa.choice_session {:items [{:description :vendor/model
                                                                          :label :vendor/model
                                                                          :search ["Hidden Label"]
                                                                          :value :vendor/model}]
                                                                 :purpose :generic
                                                                 :query :hidden
                                                                 :title :Flat
                                                                 :views [:all]}
                                                                db))
                                    (local flat-rows
                                           (. (misa.choice_rows flat db) 1
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
                                           (misa.choice_session {:items [{:value :one}
                                                                         {:value :two}
                                                                         {:value :three}]
                                                                 :preference_scope :models
                                                                 :title :Models
                                                                 :views [:browse]}
                                                                history))
                                    (local browse-rows
                                           (. (misa.choice_rows browse history)
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
                                    (set browse (misa.choice_refresh browse history))
                                    (each [_ row (ipairs (. (misa.choice_rows browse
                                                                              history)
                                                            1 :rows))]
                                      (assert (= row.section nil)
                                              "search kept recent grouping"))
                                    (local panels
                                           (misa.choice_session {:purpose :generic
                                                                 :title :Panels
                                                                 :view_definitions [{:id :one
                                                                                     :items [{:value :first}]}
                                                                                    {:id :two
                                                                                     :items [{:value :second}]}]}
                                                                db))
                                    (assert (= (misa.choice_positional panels
                                                                       :option_2_1
                                                                       1 5)
                                               nil)
                                            "hidden positional bank activated")
                                    (assert (= (misa.choice_positional panels
                                                                       :option_1_2
                                                                       1 1)
                                               nil)
                                            "hidden positional slot activated")
                                    (assert (= (. (misa.choice_positional panels
                                                                          :option_2_1
                                                                          2 5)
                                                  :value)
                                               :second)
                                            "visible positional bank did not activate")
                                    (local narrow
                                           (misa.choice_picker_layout panels db
                                                                      {:columns 57
                                                                       :lines 8}))
                                    (assert (and (= narrow.panel_count 1)
                                                 (= narrow.targets.option_2_1
                                                    nil))
                                            "nonrendered picker panel retained an active hotkey")
                                    (local wrapped
                                           (misa.choice_session {:items [{:label (string.rep "long "
                                                                                             20)
                                                                          :value :one}
                                                                         {:value :two}]
                                                                 :title :Wrapped
                                                                 :views [:all]}
                                                                db))
                                    (local visible
                                           (misa.choice_picker_layout wrapped
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
                                           (misa.choice_session {:items [{:label (string.rep "long "
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
                                    (set selected (. (misa.choice_input selected {:action :next}
                                                       db) :session))
                                    (local measured
                                           (misa.choice_picker_layout selected
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
                                                 (. measured.columns 1
                                                    :overflow))
                                            (.. "wrapped focus or overflow indicator was lost: "
                                                (length (. measured.columns 1
                                                           :rows))
                                                " " (tostring found) " "
                                                (tostring (. measured.columns 1
                                                             :overflow))))
                                    (local styled
                                           (misa.choice_row_lines {:hotkey :alt+1
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
                                           (misa.choice_picker_layout flat db
                                                                      {:available_lines 2
                                                                       :columns 30}))
                                    (assert (and (= (length (. no-budget.columns
                                                               1 :rows))
                                                    0)
                                                 (= (next no-budget.targets)
                                                    nil))
                                            "zero panel budget exposed a row or target")
                                    (local prefixed
                                           (misa.choice_session {:input_prefix "/model "
                                                                 :items [{:value :vendor/model}]
                                                                 :query :vendor/
                                                                 :title :Arguments
                                                                 :views [:all]}
                                                                db))
                                    (local prefixed-layout
                                           (misa.choice_picker_layout prefixed
                                                                      db
                                                                      {:columns 40
                                                                       :lines 8}))
                                    (assert (and (= prefixed-layout.input.text
                                                    "/model vendor/")
                                                 (= prefixed-layout.input.cursor
                                                    (length prefixed-layout.input.text)))
                                            "narrowed picker omitted its canonical command prefix")
                                    (var parent
                                           (misa.choice_session {:purpose :generic
                                                                 :title :Parent
                                                                 :view_definitions [{:id :parent
                                                                                     :items [{:value :parent}]}]}
                                                                db))
                                    (assert (and (and (= parent.selected nil)
                                                      (= parent.preference_scope
                                                         nil))
                                                 (not= parent.custom_views nil)))
                                    (set parent (. (misa.choice_accept parent
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
                                    (set parent (. (misa.choice_input parent {:action :cancel}
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
                                    (assert (= (misa.choice_action {:key :alt+x
                                                                    :kind :key})
                                               :open_overlay)
                                            "configured shared action was not resolved")
                                    (assert (= (misa.choice_hint :option_1_1)
                                               :alt+z)
                                            "configured shared positional hint was not resolved")
                                    (set ordered (misa.choice_replace_view ordered :browse
                                                              db))
                                    (local fresh
                                           (misa.choice_session {:items [{:value :vendor/one}]
                                                                 :purpose :models
                                                                 :title :Models}
                                                                db))
                                    (assert (= (. fresh.view_ids 1) :frecency)
                                            "view replacement leaked beyond its session")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text "choice contracts"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
