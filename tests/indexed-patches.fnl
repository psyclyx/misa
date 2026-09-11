;; Indexed and appended patch controls. `misa.at` writes one array element and
;; `misa.append` grows the array; both validate before they reach state, keep
;; the target's identity when nothing changes, and never mutate their inputs.
(local fennel (require :fennel))
(local G (require :tests.generators))
(local say print)
(local json-null {})
(local state ((fennel.dofile :src/lua_runtime/state.fnl) json-null))

(fn copy [value]
  (if (and (= (type value) :table) (not= value json-null))
      (let [result {}]
        (each [key item (pairs value)]
          (tset result key (copy item)))
        result)
      value))

(fn same? [left right]
  "Compare two data values structurally."
  (if (= left right) true
      (or (not= (type left) :table) (not= (type right) :table)) false
      (or (= left json-null) (= right json-null)) false
      (do
        (var equal true)
        (each [key value (pairs left)]
          (when (not (same? value (. right key)))
            (set equal false)))
        (each [key value (pairs right)]
          (when (not (same? value (. left key)))
            (set equal false)))
        equal)))

(local input {:items [{:id 1} {:id 2} {:id 3}]
              :left {:value 1 :keep true}
              :name :base})

(fn rejects [patch message]
  (assert (not (pcall state.patch input patch)) message)
  (assert (same? input {:items [{:id 1} {:id 2} {:id 3}]
                        :left {:value 1 :keep true}
                        :name :base})
          "a rejected patch changed the input state"))

;; One element is replaced in place; every other element and branch is shared.
(local written (state.patch input {:items (state.at 2 {:id 9})}))
(assert (= (length written.items) 3)
        "an indexed write changed the array length")
(assert (= (. written.items 2 :id) 9)
        "an indexed write did not replace the element")

(assert (= (. written.items 1) (. input.items 1))
        "an indexed write copied an unchanged element")

(assert (= (. written.items 3) (. input.items 3))
        "an indexed write copied an unchanged element")

(assert (= written.left input.left)
        "an indexed write replaced an unrelated branch")

;; Writing one past the end appends; writing further is rejected.
(assert (= (length (. (state.patch input {:items (state.at 4 {:id 4})}) :items))
           4) "an index one past the end did not append")

(rejects {:items (state.at 5 {:id 9})} "an index beyond the end was accepted")

;; Appending grows the array and keeps earlier elements by identity.
(local appended (state.patch input {:items (state.append {:id 4})}))
(assert (= (length appended.items) 4) "an append did not grow the array")
(assert (= (. appended.items 4 :id) 4) "an append did not write the value")
(assert (= (. appended.items 1) (. input.items 1))
        "an append copied an element")
(assert (= (length (. (state.patch appended {:items (state.append {:id 5})})
                      :items)) 5)
        "a second append did not grow the array")

;; An empty or absent target is an empty array.
(assert (= (. (state.patch {:items []} {:items (state.append 1)}) :items 1) 1)
        "an append did not accept an empty array")

(assert (= (. (state.patch {} {:items (state.at 1 :first)}) :items 1) :first)
        "an indexed write did not accept an absent array")

;; An in-range write is idempotent by identity; appending is the exception.
(assert (= (. (state.patch written {:items (state.at 2 {:id 9})}) :items)
           written.items)
        "an unchanged indexed write did not preserve array identity")

(let [token (state.append {:id 7})]
  (assert (= (length (. (state.patch input {:items token}) :items)) 4)
          "a patch token was consumed")
  (assert (= (length (. (state.patch input {:items token}) :items)) 4)
          "a patch token was mutated"))

;; Nested paths, and targets that are not arrays.
(assert (= (. (state.patch {:box {:items [1 2]}}
                           {:box {:items (state.append 3)}}) :box
              :items 3) 3)
        "an indexed write did not apply through a nested map")

(rejects {:left (state.at 1 :value)} "an indexed write accepted a map target")

;; Construction and shape validation.
(each [_ index (ipairs [-1 0 1.5 math.huge])]
  (assert (not (pcall state.at index :value))
          "an indexed patch accepted an invalid index"))

(assert (not (pcall state.at :2 :value))
        "an indexed patch accepted a string index")

(assert (not (pcall state.at 1 nil)) "an indexed patch accepted a nil value")
(assert (not (pcall state.append nil)) "an append accepted a nil value")
(rejects {:items (state.at 2 (fn [] nil))}
         "an indexed write accepted a function")
(rejects {:items (state.at 2 (/ 0 0))}
         "an indexed write accepted a non-finite number")

(rejects {:items (state.at 2 state.delete)}
         "an indexed write accepted a patch control")

(let [cycle {}]
  (tset cycle :self cycle)
  (rejects {:items (state.append cycle)} "an append accepted a cycle"))

(rejects {:items (state.at 1 (setmetatable {} {:__index input}))}
         "an indexed write accepted a metatable")

(rejects {:items (state.replace [(state.append 1)])}
         "an append was accepted inside replacement data")

(rejects {:items (state.replace (state.at 1 1))}
         "an indexed patch was accepted inside replacement data")

(rejects {:items [(state.append 1)]} "an append was accepted as sequence data")

(assert (not (pcall state.patch input (state.append 1)))
        "a patch control was accepted as the root patch")

(let [deep {}]
  (var cursor deep)
  (for [_ 1 130]
    (set cursor.child {})
    (set cursor cursor.child))
  (rejects {:items (state.at 1 deep)}))

;; Generated in-range and out-of-range writes against an independent reference.
(local scalar (G.one_of [G.boolean
                         (G.integer -100 100)
                         (G.elements ["" :text json-null])]))

(local data (G.one_of [scalar (G.vector scalar 1)]))
(local failure (G.for_all (G.tuple [(G.vector data) (G.integer -2 6) data])
                          (fn [generated]
                            (local items (copy (. generated 1)))
                            (local index (. generated 2))
                            (local value (copy (. generated 3)))
                            (local before (copy items))
                            (local target {: items})
                            (local in-range
                                   (and (>= index 1)
                                        (<= index (+ (length items) 1))))
                            (local (ok result)
                                   (pcall (fn []
                                            (state.patch target
                                                         {:items (state.at index
                                                                           value)}))))
                            (if (not in-range)
                                (assert (not ok)
                                        "an out-of-range index was accepted")
                                (do
                                  (assert ok
                                          (.. "an in-range index was rejected: "
                                              (tostring result)))
                                  (assert (= (length result.items)
                                             (math.max (length items) index))
                                          "an indexed write changed the length")
                                  (each [position prior (ipairs items)]
                                    (when (not= position index)
                                      (assert (= (. result.items position)
                                                 prior)
                                              "an indexed write copied an unchanged element")))
                                  (assert (same? (. result.items index) value)
                                          "an indexed write stored a different value")
                                  (assert (= target.items items)
                                          "an indexed write replaced the input array")
                                  (assert (same? target.items before)
                                          "an indexed write mutated its input")
                                  (assert (= (. (state.patch result
                                                             {:items (state.at index
                                                                               value)})
                                                :items)
                                             result.items)
                                          "an indexed write is not identity-idempotent"))))
                          {:seed (or (tonumber (os.getenv :MISA_PROPERTY_SEED))
                                     1729)
                           :cases 400
                           :size 5}))

(assert (not failure) (and failure (fennel.view failure)))

(say "indexed patch contracts passed")
