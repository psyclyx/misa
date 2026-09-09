(fn setup [catalogs]
  "Install service data into an isolated benchmark stub."
  (each [name value (pairs (or catalogs.services {}))]
    (let [parts (icollect [part (name:gmatch "[^.]+")] part)]
      (var owner _G.misa)
      (for [index 1 (- (length parts) 1)]
        (let [part (. parts index)]
          (when (not (. owner part)) (tset owner part {}))
          (set owner (. owner part))))
      (tset owner (. parts (length parts)) value))))
