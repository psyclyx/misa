(local styles (require :misa.ui.themes.styles))

;; Semantic theme data and style composition. Components name roles; this service
;; alone resolves those names to the closed native style record.

(fn themes-lookup [config normalized db]
  "Resolve the currently selected theme."
  (assert (and (= (type db) :table) (= (type db.themes) :table))
          "theme resolution requires initialized db")
  (let [source (assert (. (misa.catalog :themes) db.themes.active)
                       (.. "unknown theme: " (tostring db.themes.active)))]
    (or (. normalized source) (let [theme (styles.normalize source config)]
                                (tset normalized source theme)
                                theme))))

(fn themes-style [db tokens]
  "Resolve semantic style tokens through the selected theme."
  (styles.compose (misa.themes.lookup db) tokens))

(fn themes-swap [db id]
  "Return a database patch selecting a theme."
  (assert (. (misa.catalog :themes) id) (.. "unknown theme: " (tostring id)))
  (misa.patch db {:themes {:active id}}))

(fn app-start [config configured db]
  "Initialize selected presentation state and request persisted choices."
  {:patch (when (not db.themes)
            {:themes {:active configured}})
   :fx (when (not= config.persist false)
         [{:completion :themes/loaded :namespace :ui.theme :type :state/load}])})

(fn themes-loaded [db event]
  "Restore a persisted theme when its implementation is available."
  (if (or (= event.found false) (= event.data misa.json-null))
      nil
      (do
        (assert (and (= (type event.data) :table)
                     (= (type event.data.active) :string))
                "invalid persisted theme")
        (when (. (misa.catalog :themes) event.data.active)
          {:patch {:themes {:active event.data.active}}}))))

(fn themes-swap-handler [config db event]
  "Select a theme and plan persistence and redraw effects."
  (let [next (misa.themes.swap db event.theme)
        fx {}]
    (when (not= config.persist false)
      (tset fx (+ (length fx) 1)
            {:data next.themes :namespace :ui.theme :type :state/save}))
    (tset fx (+ (length fx) 1) {:event {:type :ui/redraw} :type :dispatch})
    {:patch {:themes (misa.replace next.themes)} : fx}))

{: app-start
 : themes-loaded
 : themes-lookup
 : themes-style
 : themes-swap
 : themes-swap-handler}
