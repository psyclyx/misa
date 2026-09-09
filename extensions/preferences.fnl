(local definitions (require :misa.definitions))

;; Choice preferences are immutable data; persistence is an explicit effect.
(fn new-state [] {:clock 0 :scopes {}})

(fn validate [preferences]
  (if (or (not= (type preferences) :table)
          (not= (type preferences.clock) :number) (< preferences.clock 0)
          (not= (% preferences.clock 1) 0)
          (not= (type preferences.scopes) :table))
      false
      (do
        (each [scope source-values (pairs preferences.scopes)]
          (when (or (not= (type scope) :string) (= scope "")
                    (not= (type source-values) :table))
            (lua "return false"))
          (each [value entry (pairs source-values)]
            (when (or (not= (type value) :string) (= value "")
                      (not= (type entry) :table)
                      (not= (type entry.favorite) :boolean)
                      (not= (type entry.uses) :number) (< entry.uses 0)
                      (not= (% entry.uses 1) 0))
              (lua "return false"))
            (when (and (not= entry.last nil)
                       (or (not= (type entry.last) :number) (< entry.last 0)
                           (not= (% entry.last 1) 0)
                           (> entry.last preferences.clock)))
              (lua "return false"))
            (when (and (> entry.uses 0) (= entry.last nil))
              (lua "return false"))))
        true)))

(fn entry [preferences scope value]
  (or (. (or (. preferences.scopes scope) {}) value) {:favorite false :uses 0}))

(fn configured-favorites [preferences configured]
  (var result preferences)
  (when configured
    (assert (= (type configured) :table)
            "config.preferences.favorites must be an object")
    (each [scope favorites (pairs configured)]
      (assert (and (= (type scope) :string) (not= scope "")
                   (= (type favorites) :table))
              "invalid configured favorites")
      (each [_ value (ipairs favorites)]
        (assert (and (= (type value) :string) (not= value ""))
                "configured favorites must be nonempty strings")
        (set result
             (misa.patch result
                         {:scopes {scope {value {:favorite true
                                                 :uses (. (entry result scope
                                                                 value)
                                                          :uses)}}}})))))
  result)

(fn use [db scope value]
  "Return preferences with a value's usage count and recency updated."
  (assert (and (= (type scope) :string) (= (type value) :string))
          "preference use requires scope and value")
  (local preferences (or db.preferences (new-state)))
  (local previous (entry preferences scope value))
  (local clock (+ preferences.clock 1))
  (misa.patch preferences
              {: clock
               :scopes {scope {value {:favorite previous.favorite
                                      :uses (+ previous.uses 1)
                                      :last clock}}}}))

(fn saved [preferences]
  {:patch {:preferences (misa.replace preferences)}
   :fx [{:type :state/save :namespace :preferences :data preferences}]})

(fn [context]
  "Build the declarations for preferences."
  (local config (or (. (or context.config {}) :preferences) {}))
  (assert (= (type config) :table) "config.preferences must be an object")
  (definitions :preferences
    [{:catalog :services :id :preferences.use :value use}
     {:catalog :events
      :value {:event :app/start
              :handler (fn [_]
                         {:fx [{:type :state/load
                                :namespace :preferences
                                :completion :preferences/loaded}]})}}
     {:catalog :events
      :value {:event :preferences/loaded
              :handler (fn [_ event]
                         (assert (= event.namespace :preferences)
                                 "invalid preference namespace")
                         (local preferences
                                (if (= event.found false)
                                    (new-state)
                                    event.data))
                         (assert (validate preferences)
                                 "invalid preference data")
                         {:patch {:preferences (misa.replace (configured-favorites preferences
                                                                                   config.favorites))}})}}
     {:catalog :events
      :value {:event :choice/used
              :handler (fn [db event]
                         (when (and (= (type event.scope) :string)
                                    (= (type event.value) :string))
                           (saved (use db event.scope event.value))))}}
     {:catalog :events
      :value {:event :preferences/toggle
              :handler (fn [db event]
                         (when (and (= (type event.scope) :string)
                                    (= (type event.value) :string))
                           (local preferences (or db.preferences (new-state)))
                           (local previous
                                  (entry preferences event.scope event.value))
                           (saved (misa.patch preferences
                                              {:scopes {event.scope {event.value {:uses previous.uses
                                                                                  :favorite (not previous.favorite)}}}}))))}}]
    {}))
