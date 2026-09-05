{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (set db.preferences
                                         {:scopes {:models {:one {:last 3
                                                                  :uses 3}
                                                            :three {:last 1
                                                                    :uses 1}
                                                            :two {:favorite true
                                                                  :last 2
                                                                  :uses 2}}}})
                                    (local items {})
                                    (each [_ id (ipairs [:one
                                                         :two
                                                         :three
                                                         :four
                                                         :five
                                                         :six])]
                                      (tset items (+ (length items) 1)
                                            {: id
                                             :label (.. :provider/ id)
                                             :preview {:summary "$2 in / $8 out per 1M"
                                                       :title id}
                                             :value id}))
                                    (local session
                                           (misa.choice_session {: items
                                                                 :preference_scope :models
                                                                 :title :Models
                                                                 :views [:browse
                                                                         :favorites]}
                                                                db))
                                    (var geometry
                                         (misa.choice_completion_layout session
                                                                        db 80 6))
                                    (assert (and (and geometry.input.hidden
                                                      (= geometry.panel_count 2))
                                                 (= (. geometry.preview.lines 1)
                                                    "$2 in / $8 out per 1M"))
                                            "compact choices lost panels or selected info")
                                    (var (recent all) (values false false))
                                    (each [_ row (ipairs (. geometry.columns 1
                                                            :rows))]
                                      (set recent
                                           (or recent (= row.section :Recent)))
                                      (set all (or all (= row.section :All))))
                                    (assert (and recent all)
                                            "compact recent section crowded out all items")
                                    (assert (and (= geometry.targets.option_1_1.value
                                                    :one)
                                                 (= geometry.targets.option_2_1.value
                                                    :two))
                                            "compact panel hotkeys disagreed with visible rows")
                                    (misa.choice_input session {:action :next}
                                                       db)
                                    (set geometry
                                         (misa.choice_completion_layout session
                                                                        db 80 6))
                                    (assert (= geometry.targets.option_1_1.value
                                               :two)
                                            "compacted recent choices lost keyboard focus")
                                    (local rendered
                                           (misa.render_component db :picker
                                                                  geometry))
                                    (assert (and (<= (length rendered.lines) 6)
                                                 (= rendered.cursor nil))
                                            "compact choice rendering exceeded the budget or acquired input focus")
                                    (var button false)
                                    (each [_ line (ipairs rendered.lines)]
                                      (each [_ span (ipairs line.spans)]
                                        (when (= span.action
                                                 :choices.option_1_1)
                                          (set button true))))
                                    (assert button
                                            "semantic choice button disappeared during rendering")
                                    (set geometry
                                         (misa.choice_completion_layout session
                                                                        db 40 6))
                                    (assert (and (= geometry.panel_count 1)
                                                 (not geometry.targets.option_2_1))
                                            "hidden panel retained a positional target")
                                    (set session.query :two)
                                    (misa.choice_refresh session db)
                                    (each [_ item (ipairs (. session.panels 1
                                                             :items))]
                                      (assert (= item.section nil)
                                              "search retained recent grouping"))
                                    (local tall
                                           (misa.choice_session {:items [{:label (string.rep "very long model "
                                                                                             20)
                                                                          :value :long}]
                                                                 :title :Long
                                                                 :views [:all]}
                                                                db))
                                    (local short
                                           (misa.choice_completion_layout tall
                                                                          db 32
                                                                          4))
                                    (assert (and short.targets.option_1_1
                                                 (= short.targets.option_1_1.value
                                                    :long))
                                            "a tall choice became unreachable in a small viewport")
                                    (local lines (. short.columns 1 :lines))
                                    (assert (= (: (. lines (length lines)
                                                     :spans 1 :text)
                                                  :sub (- (length "…")))
                                               "…")
                                            "omitted choice content had no visible continuation marker")
                                    {: db
                                     :fx [{:lines [{:spans [{:text "compact choices"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
