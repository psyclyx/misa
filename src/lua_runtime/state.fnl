;; Persistent updates over ordinary Lua tables. Callers treat state as immutable.
;; Empty patches merge; clearing a collection requires explicit replacement.
(fn factory [json-null]
  "Create persistent update operations with an explicit JSON null sentinel."
  (local delete {})
  (local replacements (setmetatable {} {:__mode :k}))
  (local indexed (setmetatable {} {:__mode :k}))
  (local max-depth 128)

  (fn shallow [value]
    (local result {})
    (each [key item (pairs value)] (tset result key item))
    result)

  (fn finite? [n]
    (and (= n n) (not= n math.huge) (not= n (- math.huge))))

  (fn key! [key]
    (assert (or (= (type key) :string)
                (and (= (type key) :number) (finite? key) (>= key 1)
                     (= (% key 1) 0))) "invalid state key"))

  (fn enter! [value active depth]
    (assert (<= depth max-depth) "maximum state nesting depth exceeded")
    (assert (not (. active value)) "cyclic state or patch")
    (assert (= (getmetatable value) nil)
            "state and patches must be ordinary tables")
    (tset active value true))

  ;; Materialize a replacement, validating its entire shape and retaining old
  ;; branches where equal. Allocate only after finding a difference; identity
  ;; never skips validation. Control values are never application data.

  (fn materialize [old value active depth]
    (assert (<= depth max-depth) "maximum state nesting depth exceeded")
    (assert (and (not= value delete) (not (. replacements value))
                 (not (. indexed value)))
            "patch control inside replacement data")
    (local kind (type value))
    (if (or (= value json-null) (= kind :nil) (= kind :string)
            (= kind :boolean))
        value
        (= kind :number)
        (do
          (assert (finite? value) "non-finite state number")
          value)
        (do
          (assert (= kind :table) "state must contain only data")
          (enter! value active depth)
          (local base (if (and (= (type old) :table) (not= old json-null)) old
                          {}))
          (var result (when (or (not= (type old) :table) (= old json-null))
                        {}))
          (each [key item (pairs value)]
            (key! key)
            (local next (materialize (. base key) item active (+ depth 1)))
            (when (not= next (. base key))
              (set result (or result (shallow base)))
              (tset result key next)))
          (each [key _ (pairs base)]
            (when (= (. value key) nil)
              (set result (or result (shallow base)))
              (tset result key nil)))
          (tset active value nil)
          (or result old))))

  ;; An indexed patch writes one array element; `index` is nil for an append.
  ;; The write keeps the target array's identity when the element is unchanged,
  ;; so re-applying an in-range indexed patch is a no-op. Appending is
  ;; therefore the one patch control that is not idempotent.

  (fn array-base [old]
    (if (and (= (type old) :table) (not= old json-null)) old {}))

  (fn dense? [value]
    (var count 0)
    (var dense true)
    (each [key _ (pairs value)]
      (set count (+ count 1))
      (when (not (and (= (type key) :number) (>= key 1) (= (% key 1) 0)))
        (set dense false)))
    (when (and dense (not= (length value) count))
      (set dense false))
    dense)

  (fn indexed-write [old entry active depth]
    (local base (array-base old))
    (assert (dense? base) "an indexed patch needs an array target")
    (local size (length base))
    (local index (or entry.index (+ size 1)))
    (assert (<= index (+ size 1))
            "indexed patch index is past the end of the array")
    (local next (materialize (. base index) entry.value active (+ depth 1)))
    (if (= next (. base index)) old
        (let [result (shallow base)]
          (tset result index next)
          result)))

  (fn sequence? [value]
    (var count 0)
    (var numeric false)
    (var named false)
    (each [key _ (pairs value)]
      (key! key)
      (set count (+ count 1))
      (if (= (type key) :number) (set numeric true) (set named true)))
    (when numeric
      (assert (not named) "patch cannot mix map and sequence keys")
      (for [index 1 count]
        (assert (not= (. value index) nil) "patch sequence must be dense")))
    numeric)

  (fn merge [old patch active depth]
    (assert (<= depth max-depth) "maximum patch nesting depth exceeded")
    (local replacement (. replacements patch))
    (local indexed-entry (. indexed patch))
    (if (= patch delete) nil replacement
        (materialize old replacement.value active depth) indexed-entry
        (indexed-write old indexed-entry active depth)
        (or (not= (type patch) :table) (= patch json-null))
        (materialize old patch active depth) (sequence? patch)
        (materialize old patch active depth)
        (do
          (enter! patch active depth)
          (local base (if (and (= (type old) :table) (not= old json-null)) old
                          {}))
          (var result nil)
          (each [key value (pairs patch)]
            (local next (merge (. base key) value active (+ depth 1)))
            (when (not= next (. base key))
              (when (not result)
                (set result {})
                (each [base-key base-value (pairs base)]
                  (tset result base-key base-value)))
              (tset result key next)))
          (tset active patch nil)
          (or result old))))

  {: delete
   :replace (fn [value]
              "Mark a value for replacement instead of recursive map merging."
              (local token {})
              (tset replacements token {: value})
              token)
   :at (fn [index value]
         "Mark one array element for replacement, or the end for an append."
         (assert (and (= (type index) :number) (finite? index) (>= index 1)
                      (= (% index 1) 0))
                 "an indexed patch needs a positive integer index")
         (assert (not= value nil)
                 "an indexed patch needs a value; rebuild the array to shorten it")
         (local token {})
         (tset indexed token {: index : value})
         token)
   :append (fn [value]
             "Mark a value for appending to the end of an array."
             (assert (not= value nil) "an appended patch needs a value")
             (local token {})
             (tset indexed token {: value})
             token)
   :patch (fn [state patch]
            "Apply a validated patch while sharing unchanged state branches."
            (assert (and (= (type state) :table) (not= state json-null))
                    "patch state must be a table")
            (assert (and (= (type patch) :table) (not= patch delete)
                         (not= patch json-null) (not (. replacements patch))
                         (not (. indexed patch)) (not (sequence? patch)))
                    "root patch must be a map")
            (merge state patch {} 0))})

factory
