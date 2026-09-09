(local definitions (require :misa.definitions))

;; Choice preferences are immutable data; persistence is an explicit effect.
(fn new-state [] {:clock 0 :scopes {}})

(fn nonnegative-integer? [value]
  (and (= (type value) :number) (>= value 0) (= (% value 1) 0)))

(fn valid-entry? [value entry clock]
  (and (= (type value) :string) (not= value "") (= (type entry) :table)
       (= (type entry.favorite) :boolean) (nonnegative-integer? entry.uses)
       (or (= entry.last nil)
           (and (nonnegative-integer? entry.last) (<= entry.last clock)))
       (or (= entry.uses 0) (not= entry.last nil))))

(fn valid? [preferences]
  "Check persisted choice preferences before they enter application state."
  (and (= (type preferences) :table) (nonnegative-integer? preferences.clock)
       (= (type preferences.scopes) :table)
       (accumulate [valid true scope entries (pairs preferences.scopes)
                    &until (not valid)]
         (and (= (type scope) :string) (not= scope "")
              (= (type entries) :table)
              (accumulate [valid true value entry (pairs entries)
                           &until (not valid)]
                (valid-entry? value entry preferences.clock))))))

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
  (let [preferences (or db.preferences (new-state))
        previous (entry preferences scope value)
        clock (+ preferences.clock 1)]
    (misa.patch preferences
                {: clock
                 :scopes {scope {value {:favorite previous.favorite
                                        :uses (+ previous.uses 1)
                                        :last clock}}}})))

(fn saved [preferences]
  {:patch {:preferences (misa.replace preferences)}
   :fx [{:type :state/save :namespace :preferences :data preferences}]})

(fn on-preferences-toggle [db event]
  (when (and (= (type event.scope) :string) (= (type event.value) :string))
    (let [preferences (or db.preferences (new-state))
          previous (entry preferences event.scope event.value)]
      (saved (misa.patch preferences
                         {:scopes {event.scope {event.value {:uses previous.uses
                                                             :favorite (not previous.favorite)}}}})))))

(fn on-choice-used [db event]
  (when (and (= (type event.scope) :string) (= (type event.value) :string))
    (saved (use db event.scope event.value))))

(fn on-app-start [_]
  {:fx [{:type :state/load
         :namespace :preferences
         :completion :preferences/loaded}]})

(fn build [context]
  "Build the declarations for preferences."
  (let [config (or (. (or context.config {}) :preferences) {})]
    (assert (= (type config) :table) "config.preferences must be an object")

    (fn on-preferences-loaded [_ event]
      (assert (= event.namespace :preferences) "invalid preference namespace")
      (let [preferences (if (= event.found false)
                            (new-state)
                            event.data)]
        (assert (valid? preferences) "invalid preference data")
        {:patch {:preferences (misa.replace (configured-favorites preferences
                                                                  config.favorites))}}))

    (definitions.build :preferences
      [{:catalog :services :id :preferences.use :value use}
       {:catalog :events :value {:event :app/start :handler on-app-start}}
       {:catalog :events
        :value {:event :preferences/loaded :handler on-preferences-loaded}}
       {:catalog :events :value {:event :choice/used :handler on-choice-used}}
       {:catalog :events
        :value {:event :preferences/toggle :handler on-preferences-toggle}}]
      {})))

{: build : valid? : use}
