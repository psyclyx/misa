;; Install pure service declarations into an isolated benchmark stub.
(fn setup [extension context]
  (local result ((if (= (type extension) :function) extension extension.build) (or context {})))
  (each [name value (pairs (or result.services {}))]
    (local parts [])
    (each [part (name:gmatch "[^.]+")]
      (table.insert parts part))
    (var owner _G.misa)
    (for [index 1 (- (length parts) 1)]
      (local part (. parts index))
      (when (not (. owner part)) (tset owner part {}))
      (set owner (. owner part)))
    (tset owner (. parts (length parts)) value)))
