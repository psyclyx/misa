(fn definitions [owner rows catalogs]
  "Build named catalogs from declaration rows without installing them.

Explicit IDs are preserved; unnamed event handlers receive an owner-local ID."
  (local result {})
  (each [kind entries (pairs (or catalogs {}))]
    (tset result kind (collect [id value (pairs entries)] id value)))
  (local counts {})
  (each [index row (ipairs rows)]
    (local kind (assert row.catalog "declaration requires a catalog"))
    (local entries (or (. result kind) {}))
    (local base (or row.id (and (= kind :events) row.value.event)))
    (assert base "declaration requires an ID")
    (local id (if (and (= kind :events) (= row.id nil))
                  (let [count (+ (or (. counts base) 0) 1)]
                    (tset counts base count)
                    (.. owner "/" base (if (= count 1) "" (.. "/" count))))
                  base))
    (assert (= (. entries id) nil) (.. "duplicate declaration: " kind "/" id))
    (tset entries id (assert row.value "declaration requires a value"))
    (tset result kind entries))
  result)

definitions
