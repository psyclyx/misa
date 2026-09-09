(local themes (require :misa.ui.themes))
(local styles (require :misa.ui.themes.styles))
(local variants (require :misa.ui.themes.default))
(local caches (setmetatable {} {:__mode :k}))

(fn settings [config] (or config.themes {}))
(fn selected [config]
  (let [id (or config.default :default)
        appearance (or config.appearance :dark)]
    (assert (or (= appearance :dark) (= appearance :light))
            "themes.appearance must be dark or light")
    (if (and (= id :default) (= appearance :light)) :light id)))

(fn lookup [db]
  (let [configuration (misa.configuration)
        cache (or (. caches configuration) {})]
    (tset caches configuration cache)
    (themes.themes-lookup (settings configuration) cache db)))

{:themes {:default variants.dark :light variants.light}
 :services {:themes.lookup lookup
            :themes.style themes.themes-style
            :themes.swap themes.themes-swap}
 :events {:themes/app/start {:event :app/start
                             :handler (fn [db _ cofx]
                                        (let [config (settings cofx.config)
                                              id (selected config)]
                                          (styles.normalize (assert (. (misa.catalog :themes)
                                                                       id)
                                                                    "unknown configured theme")
                                                            config)
                                          (themes.app-start config id db)))}
          :themes/themes/loaded {:event :themes/loaded
                                 :handler themes.themes-loaded}
          :themes/themes/swap {:event :themes/swap
                               :handler (fn [db event cofx]
                                          (themes.themes-swap-handler (settings cofx.config)
                                                                      db event))}}
 :validators {:themes (fn [_ theme] (styles.normalize theme {}) nil)}}
