{:setup (fn []
          (misa.reg_command {:description "terminal command"
                             :event :test/alpha
                             :name :/alpha})
          (misa.reg_command {:completion :history-arguments
                             :description "argument command"
                             :event :test/zulu
                             :name :/zulu
                             :preference_scope :history-arguments})
          (misa.reg_completion :history-arguments {:id :one-id :value :one})
          (misa.reg_event :test/alpha
                          (fn [db]
                            {: db :fx [{:type :terminal/read}]}))
          (misa.reg_event :test/zulu
                          (fn [db event]
                            (assert (= event.arguments :one))
                            (local used
                                   (. db.preferences.scopes.commands
                                      "/zulu one" :uses))
                            (if (= used 1)
                                {: db
                                 :fx [{:event {:scope :commands
                                               :type :preferences/toggle
                                               :value "/zulu one"}
                                       :type :dispatch}
                                      {:event {:type :test/history}
                                       :type :dispatch}]}
                                (do
                                  (assert (= used 2)
                                          "picker replay counted command usage twice")
                                  (assert (= (. db.preferences.scopes
                                                :history-arguments :one-id :uses)
                                             2)
                                          "argument usage did not share the invocation owner")
                                  {: db
                                   :fx [{:lines [{:spans [{:style {:foreground :default}
                                                           :text "command history"}]}]
                                         :type :view/commit}
                                        {:type :app/quit}]}))))
          (misa.reg_event :test/history
                          (fn [db]
                            (assert (= (. db.preferences.scopes.commands
                                          :/alpha :uses)
                                       2)
                                    "typed terminal command usage was lost")
                            (assert (= (. db.preferences.scopes
                                          :history-arguments :one-id :uses)
                                       1)
                                    "typed argument usage was lost")
                            (local session (misa.omnipicker_session db ""))
                            (local rows (misa.choice_rows session db))
                            (assert (and (and (= session.preference_scope
                                                 :commands)
                                              (= (. rows 1 :rows 1 :id)
                                                 "/zulu one"))
                                         (= (. rows 1 :rows 1 :section) :Recent))
                                    "typed invocation did not lead Recent")
                            (assert (and (= (length (. rows 2 :rows)) 1)
                                         (= (. rows 2 :rows 1 :id) "/zulu one"))
                                    "canonical favorite was not available")
                            (local counts {})
                            (each [_ item (ipairs session.items)]
                              (tset counts item.id
                                    (+ (or (. counts item.id) 0) 1)))
                            (assert (and (= (. counts :/alpha) 1)
                                         (= (. counts "/zulu one") 1))
                                    "command source duplicated a canonical invocation")
                            (set db.history_frame
                                 (misa.choice_session {:items [{:value :parent}]
                                                       :purpose :generic
                                                       :title :Parent}
                                                      db))
                            (misa.choice_accept db.history_frame
                                                {:narrow {:items [{:value :child}]
                                                          :preference_scope :child
                                                          :purpose :generic
                                                          :selected :child
                                                          :title :Child}}
                                                db)
                            {: db
                             :fx [{:event {:type :test/restored}
                                   :type :dispatch}]}))
          (misa.reg_event :test/restored
                          (fn [db]
                            (misa.choice_input db.history_frame
                                               {:action :cancel} db)
                            (assert (and (and (= db.history_frame.selected nil)
                                              (= db.history_frame.preference_scope
                                                 nil))
                                         (= db.history_frame.custom_views nil))
                                    "snapshot cloning changed absent narrow-frame values into tables")
                            {: db
                             :fx [{:event {:completion :omnipicker/selected
                                           :id :omnipicker
                                           :session (misa.omnipicker_session db
                                                                             "zulu one")
                                           :title :Commands
                                           :token :history
                                           :type :picker/open}
                                   :type :dispatch}
                                  {:event {:kind :alt
                                           :text :1
                                           :type :picker/input}
                                   :type :dispatch}]}))
          nil)}

