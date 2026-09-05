;; Consume declarative parser/service setup outside the application runtime.
;; Historical Lua baselines may still install their services and return nil.
(fn setup [extension context]
  (local result (extension.setup context))
  (each [_ effect (ipairs (or (and result result.fx) []))]
    (assert (= effect.type :register/service)
            "benchmark setup expects service effects")
    (local parts [])
    (each [part (effect.name:gmatch "[^.]+")]
      (table.insert parts part))
    (var owner _G.misa)
    (for [index 1 (- (length parts) 1)]
      (local part (. parts index))
      (when (not (. owner part)) (tset owner part {}))
      (set owner (. owner part)))
    (tset owner (. parts (length parts)) effect.value)))
