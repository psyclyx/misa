;; Persistent updates over ordinary Lua tables. Callers treat state as immutable.
;; Empty patches merge; clearing a collection requires explicit replacement.
(fn factory [json-null]
  (local delete {})
  (local replacements (setmetatable {} {:__mode :k}))
  (local max-depth 128)

  (fn finite? [n]
    (and (= n n) (not= n math.huge) (not= n (- math.huge))))

  (fn key! [key]
    (assert (or (= (type key) :string)
                (and (= (type key) :number) (finite? key)
                     (>= key 1) (= (% key 1) 0)))
            "invalid state key"))

  (fn enter! [value active depth]
    (assert (<= depth max-depth) "maximum state nesting depth exceeded")
    (assert (not (. active value)) "cyclic state or patch")
    (assert (= (getmetatable value) nil) "state and patches must be ordinary tables")
    (tset active value true))

  ;; Materialize a replacement, validating its entire shape and retaining old
  ;; branches where equal. Control values are never application data.
  (fn materialize [old value active depth]
    (assert (<= depth max-depth) "maximum state nesting depth exceeded")
    (assert (and (not= value delete) (not (. replacements value)))
            "patch control inside replacement data")
    (local kind (type value))
    (if (or (= value json-null) (= kind :nil)
            (= kind :string) (= kind :boolean)) value
        (= kind :number) (do (assert (finite? value) "non-finite state number") value)
        (do
          (assert (= kind :table) "state must contain only data")
          (enter! value active depth)
          (local base (if (and (= (type old) :table) (not= old json-null)) old {}))
          (local result {})
          (var changed (or (not= (type old) :table) (= old json-null)))
          (each [key item (pairs value)]
            (key! key)
            (local next (materialize (. base key) item active (+ depth 1)))
            (tset result key next)
            (when (not= next (. base key)) (set changed true)))
          (each [key _ (pairs base)]
            (when (= (. value key) nil) (set changed true)))
          (tset active value nil)
          (if changed result old))))

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
    (if (= patch delete) nil
        replacement (materialize old replacement.value active depth)
        (or (not= (type patch) :table) (= patch json-null))
        (materialize old patch active depth)
        (sequence? patch) (materialize old patch active depth)
        (do
          (enter! patch active depth)
          (local base (if (and (= (type old) :table) (not= old json-null)) old {}))
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
              (local token {})
              (tset replacements token {: value})
              token)
   :patch (fn [state patch]
            (assert (and (= (type state) :table) (not= state json-null))
                    "patch state must be a table")
            (assert (and (= (type patch) :table) (not= patch delete)
                         (not= patch json-null) (not (. replacements patch))
                         (not (sequence? patch)))
                    "root patch must be a map")
            (merge state patch {} 0))})

factory
