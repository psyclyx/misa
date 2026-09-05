;; Generic choice preferences backed by the native namespaced state effects.

(fn new-state [] {:clock 0 :scopes {}})

(fn state [db]
  (set db.preferences (or db.preferences (new-state)))
  db.preferences)

(fn scope-state [preferences scope]
  (tset preferences.scopes scope (or (. preferences.scopes scope) {}))
  (. preferences.scopes scope))

(fn validate [preferences]
  (if (or (or (or (or (not= (type preferences) :table)
                      (not= (type preferences.clock) :number))
                  (< preferences.clock 0))
              (not= (% preferences.clock 1) 0))
          (not= (type preferences.scopes) :table))
      false
      (do
        (each [scope ___values___ (pairs preferences.scopes)]
          (when (or (or (not= (type scope) :string) (= scope ""))
                    (not= (type ___values___) :table))
            (lua "return false"))
          (each [value entry (pairs ___values___)]
            (when (or (or (or (or (or (or (not= (type value) :string)
                                          (= value ""))
                                      (not= (type entry) :table))
                                  (not= (type entry.favorite) :boolean))
                              (not= (type entry.uses) :number))
                          (< entry.uses 0))
                      (not= (% entry.uses 1) 0))
              (lua "return false"))
            (when (and (not= entry.last nil)
                       (or (or (or (not= (type entry.last) :number)
                                   (< entry.last 0))
                               (not= (% entry.last 1) 0))
                           (> entry.last preferences.clock)))
              (lua "return false"))
            (when (and (> entry.uses 0) (= entry.last nil))
              (lua "return false"))))
        true)))

(fn merge-configured-favorites [preferences configured]
  (if (= configured nil) nil (do
                               (assert (= (type configured) :table)
                                       "config.preferences.favorites must be an object")
                               (each [scope favorites (pairs configured)]
                                 (assert (and (and (= (type scope) :string)
                                                   (not= scope ""))
                                              (= (type favorites) :table))
                                         "invalid configured favorites")
                                 (local ___values___
                                        (scope-state preferences scope))
                                 (each [_ value (ipairs favorites)]
                                   (assert (and (= (type value) :string)
                                                (not= value ""))
                                           "configured favorites must be nonempty strings")
                                   (local entry
                                          (or (. ___values___ value) {:uses 0}))
                                   (set entry.favorite true)
                                   (tset ___values___ value entry)))
                               nil)))

{:setup (fn [context]
          (local preferences-config
                 (or (and (= (type context.config) :table)
                          context.config.preferences) nil))
          (when (not= preferences-config nil)
            (assert (= (type preferences-config) :table)
                    "config.preferences must be an object"))
          (local configured-favorites
                 (or (and preferences-config preferences-config.favorites) nil))
          (misa.reg_event :app/start
                          (fn [db]
                            {: db
                             :fx [{:completion :preferences/loaded
                                   :namespace :preferences
                                   :type :state/load}]}))
          (misa.reg_event :preferences/loaded
                          (fn [db event]
                            (assert (= event.namespace :preferences)
                                    "invalid preference namespace")
                            (local preferences
                                   (or (and (= event.found false) (new-state))
                                       event.data))
                            (assert (validate preferences)
                                    "invalid preference data")
                            (merge-configured-favorites preferences
                                                        configured-favorites)
                            (set db.preferences preferences)
                            {: db}))

          (fn misa.preference_use [db scope value]
            (assert (and (= (type scope) :string) (= (type value) :string))
                    "preference use requires scope and value")
            (local preferences (state db))
            (set preferences.clock (+ preferences.clock 1))
            (local ___values___ (scope-state preferences scope))
            (local entry (or (. ___values___ value) {:favorite false :uses 0}))
            (set (entry.uses entry.last)
                 (values (+ entry.uses 1) preferences.clock))
            (tset ___values___ value entry)
            {:data preferences :namespace :preferences :type :state/save})

          (misa.reg_event :choice/used
                          (fn [db event]
                            (if (or (not= (type event.scope) :string)
                                    (not= (type event.value) :string))
                                nil
                                {: db
                                 :fx [(misa.preference_use db event.scope
                                                           event.value)]})))
          (misa.reg_event :preferences/toggle
                          (fn [db event]
                            (if (or (not= (type event.scope) :string)
                                    (not= (type event.value) :string))
                                nil
                                (do
                                  (local preferences (state db))
                                  (local ___values___
                                         (scope-state preferences event.scope))
                                  (local entry
                                         (or (. ___values___ event.value)
                                             {:uses 0}))
                                  (set entry.favorite
                                       (not (= entry.favorite true)))
                                  (tset ___values___ event.value entry)
                                  {: db
                                   :fx [{:data preferences
                                         :namespace :preferences
                                         :type :state/save}]}))))
          nil)}

