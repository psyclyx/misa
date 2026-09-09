(local definitions (require :misa.definitions))

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:description "terminal command"
                                 :event :test/alpha
                                 :name :/alpha}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        (let [definition {:completion :history-arguments
                                 :description "argument command"
                                 :event :test/zulu
                                 :name :/zulu
                                 :preference_scope :history-arguments}] {:catalog :commands :id (. definition :name) :value definition}))
          (table.insert declarations
                        {:catalog :completions :id (.. :history-arguments "/" (. {:id :one-id :value :one} :value)) :value {:group :history-arguments :value {:id :one-id :value :one}}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/alpha :handler (fn [db]
                                    {:fx [{:type :terminal/read}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/zulu :handler (fn [db event]
                                    (assert (= event.arguments :one))
                                    (local used
                                           (. db.preferences.scopes.commands
                                              "/zulu one" :uses))
                                    (if (= used 1)
                                        {:fx [{:event {:scope :commands
                                                       :type :preferences/toggle
                                                       :value "/zulu one"}
                                               :type :dispatch}
                                              {:event {:type :test/history}
                                               :type :dispatch}]}
                                        (do
                                          (assert (= used 2)
                                                  "picker replay counted command usage twice")
                                          (assert (= (. db.preferences.scopes
                                                        :history-arguments
                                                        :one-id :uses)
                                                     2)
                                                  "argument usage did not share the invocation owner")
                                          {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                                   :text "command history"}]}]
                                                 :type :view/commit}
                                                {:type :app/quit}]})))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/history :handler (fn [db]
                                    (assert (= (. db.preferences.scopes.commands
                                                  :/alpha :uses)
                                               2)
                                            "typed terminal command usage was lost")
                                    (assert (= (. db.preferences.scopes
                                                  :history-arguments :one-id
                                                  :uses)
                                               1)
                                            "typed argument usage was lost")
                                    (var session
                                           (misa.picker.session db ""))
                                    (local rows (misa.choices.rows session db))
                                    (assert (and (and (= session.preference_scope
                                                         :commands)
                                                      (= (. rows 1 :rows 1 :id)
                                                         "/zulu one"))
                                                 (= (. rows 1 :rows 1 :section)
                                                    :Recent))
                                            "typed invocation did not lead Recent")
                                    (assert (and (= (length (. rows 2 :rows)) 1)
                                                 (= (. rows 2 :rows 1 :id)
                                                    "/zulu one"))
                                            "canonical favorite was not available")
                                    (local counts {})
                                    (each [_ item (ipairs session.items)]
                                      (tset counts item.id
                                            (+ (or (. counts item.id) 0) 1)))
                                    (assert (and (= (. counts :/alpha) 1)
                                                 (= (. counts "/zulu one") 1))
                                            "command source duplicated a canonical invocation")
                                    (local parent
                                         (misa.choices.session {:items [{:value :parent}]
                                                               :purpose :generic
                                                               :title :Parent}
                                                              db))
                                    (local frame (. (misa.choices.accept parent
                                                        {:narrow {:items [{:value :child}]
                                                                  :preference_scope :child
                                                                  :purpose :generic
                                                                  :selected :child
                                                                  :title :Child}}
                                                        db) :session))
                                    {:patch {:history_frame (misa.replace frame)}
                                     :fx [{:event {:type :test/restored}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/restored :handler (fn [db]
                                    (local frame (. (misa.choices.input db.history_frame
                                                       {:action :cancel} db) :session))
                                    (assert (and (and (= frame.selected
                                                         nil)
                                                      (= frame.preference_scope
                                                         nil))
                                                 (= frame.custom_views
                                                    nil))
                                            "snapshot cloning changed absent narrow-frame values into tables")
                                    {:patch {:history_frame (misa.replace frame)}
                                     :fx [{:event {:completion :omnipicker/selected
                                                   :id :omnipicker
                                                   :session (misa.picker.session db
                                                                                     "zulu one")
                                                   :title :Commands
                                                   :token :history
                                                   :type :picker/open}
                                           :type :dispatch}
                                          {:event {:kind :alt
                                                   :text :j
                                                   :type :picker/input}
                                           :type :dispatch}]})}})
          nil
          (definitions.build :tests.command-history declarations {}))
