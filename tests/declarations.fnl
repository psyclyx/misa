(fn collect-declarations [owner rows catalogs]
  "Build named catalogs from declaration rows without installing them.

Explicit IDs are preserved; unnamed event handlers receive an owner-local ID."
  (let [result {}]
    (each [kind entries (pairs (or catalogs {}))]
      (tset result kind (collect [id value (pairs entries)] id value)))
    (let [counts {}]
      (each [index row (ipairs rows)]
        (let [kind (assert row.catalog "declaration requires a catalog")
              entries (or (. result kind) {})
              base (or row.id (and (= kind :events) row.value.event))]
          (assert base "declaration requires an ID")
          (let [id (if (and (= kind :events) (= row.id nil))
                       (let [count (+ (or (. counts base) 0) 1)]
                         (tset counts base count)
                         (.. owner "/" base (if (= count 1) "" (.. "/" count))))
                       base)]
            (assert (= (. entries id) nil)
                    (.. "duplicate declaration: " kind "/" id))
            (tset entries id (assert row.value "declaration requires a value"))
            (tset result kind entries))))
      result)))

{:collect collect-declarations}
