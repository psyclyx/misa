(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(misa._setup (fennel.dofile :extensions/json.fnl) {})
(local specs ((. (fennel.dofile :extensions/messages.fnl) :setup) {:config {}}))
(var (viewport scroll) nil)
(each [_ spec (ipairs specs.fx)]
  (when (= spec.name :transcript_viewport) (set viewport spec.value))
  (when (= spec.name :messages/scroll) (set scroll spec.handler)))
(var projections 0)
(set misa.transcript_projection
     (fn [db]
       (set projections (+ projections 1))
       (icollect [index (ipairs db.rows)]
         {:spans [{:text (tostring index)}] :selected (= index db.selected_row)})))
(set misa.selection_projection (fn [db] db.selected))
(fn snapshot [count selected-row]
  {:messages {} :rows (fcollect [i 1 count] i) :selected_row selected-row
   :selected (when selected-row {:id :doc :first selected-row :last (+ selected-row 1)})})
(fn project [db room]
  (local before (misa.json.encode db))
  (local result (viewport db {:columns 80} room))
  (assert (= before (misa.json.encode db)))
  result)
(local original (snapshot 100))
(local expected (project original 10))
(assert (= expected.first 91))
(project (snapshot 200 12) 40)
(assert (= (misa.json.encode (project original 10)) (misa.json.encode expected))
        "a different speculative render changed the viewport")
(local selected (snapshot 100 12))
(assert (= (. (project selected 10) :first) 12))
(set misa.ui_regions (fn [db terminal]
                      [{:id :transcript :viewport (viewport db {:columns terminal.columns} (- terminal.lines 4))}]))
(fn move [db delta]
  (local before (misa.json.encode db))
  (local result (scroll db {: delta} {:terminal {:lines 14 :columns 80}}))
  (assert (= before (misa.json.encode db)))
  (misa.patch db (or result.patch {})))
(local moved (move selected -5))
(assert (= moved.messages.top 17))
(assert (= (. (project moved 10) :first) 17) "explicit scroll was overridden by selection")
(assert (= (. (project selected 10) :first) 12) "rollback retained a later scroll")
(local changed-selection (misa.patch moved {:selected_row 50 :selected {:first 50 :last 51}}))
(assert (= (. (project changed-selection 10) :first) 41))
(local tail (move original 5))
(assert (= tail.messages.top 86))
(assert (= (. (project (misa.patch tail {:rows (misa.replace (. (snapshot 150) :rows))}) 10) :first) 86))
(local failure
       (G.for_all (G.tuple [(G.elements [0 1 10 100]) (G.elements [0 1 10 40])])
                  (fn [sample]
                    (local count (. sample 1))
                    (local room (. sample 2))
                    (local db (snapshot count))
                    (local result (project db room))
                    (assert (<= (length result.lines) room))
                    (assert (>= result.first 1))
                    (project (snapshot 200 7) 3)
                    (assert (= (misa.json.encode result) (misa.json.encode (project db room)))))))
(assert (not failure) (and failure (fennel.view failure)))
(set projections 0)
(project original 10)
(assert (= projections 1) "viewport projected transcript more than once")
(set projections 0)
(move original 3)
(assert (= projections 1) "scroll measured the transcript more than once")
(local original-projection misa.transcript_projection)
(set misa.transcript_projection
     (fn [db]
       (local lines (original-projection db))
       (set (. lines 1 :selected) true)
       (set (. lines 1 :selection_id) :earlier-document)
       (set (. lines 12 :selection_id) :doc)
       lines))
(assert (= (. (project selected 10) :first) 12)
        "cross-document range revealed the anchor instead of the focused document")
(set misa.transcript_projection original-projection)
(set misa.transcript_projection
     (fn [db]
       (local lines (original-projection db))
       (set (. lines 12 :selected) false)
       (set (. lines 12 :selection_anchor) true)
       (set (. lines 12 :selection_id) :doc)
       (set (. lines 1 :selection_anchor) true)
       (set (. lines 1 :selection_id) :earlier-document)
       lines))
(assert (= (. (project selected 10) :first) 12)
        "an unpainted selection anchor was not revealed")
(set misa.transcript_projection original-projection)
;; Width/detail changes alter row counts before the anchor, not its identity.
(set misa.transcript_projection
     (fn [db context]
       (local lines [])
       (for [block 1 30]
         (for [row 1 (if (= context.columns 40) 4 2)]
           (table.insert lines {:transcript_id (tostring block) :source_start 0
                                :spans [{:text (.. block ":" row)}]})))
       lines))
(local anchored (move original 15))
(local before-resize (viewport anchored {:columns 80} 10))
(local after-resize (viewport anchored {:columns 40} 10))
(assert (= (. before-resize.lines 1 :transcript_id) (. after-resize.lines 1 :transcript_id))
        "resize lost the scrolled content block")
(local followed (viewport original {:columns 40} 10))
(assert (= followed.first 111) "tail-following acquired a content anchor")
(set misa.transcript_projection original-projection)
;; A broad source range (such as a block heading) can contain the source byte
;; of a later row. It must not steal that row's explicit scroll anchor.
(set misa.transcript_projection
     (fn [db]
       (local lines (fcollect [i 1 30]
         {:transcript_id :block :source_part :body
          :spans [{:text (tostring i) :source true
                   :source_start (if (= i 1) 0 (* i 10))
                   :source_end (if (= i 1) 1000 (+ (* i 10) 10 (if db.growing 5 0)))}]}))
       (when db.leading (table.insert lines 1 {:transcript_id :new :spans [{:text :new}]}))
       lines))
(var walking {:messages {}})
(for [i 1 6]
  (set walking (move walking 1))
  (assert (= (. (project walking 10) :first) (- 21 i))
          "a containing source range overrode explicit upward scrolling")
  (assert (= (. (project (misa.patch walking {:growing true}) 10) :first) (- 21 i))
          "extending a source range without moving its row triggered re-anchoring")
  (assert (= (. (project (misa.patch walking {:leading true}) 10) :first) (- 22 i))
          "content inserted above the viewport lost the exact source anchor"))
(for [i 1 6]
  (set walking (move walking -1))
  (assert (= (. (project walking 10) :first) (+ 15 i))
          "reflow anchoring overrode explicit downward scrolling"))
(set misa.transcript_projection original-projection)
;; Streaming and scroll events share the same rule: preserve the visible
;; content first, then apply the requested row delta exactly once.
(fn row [id source last]
  {:transcript_id id :spans [{:text id :source true :source_start (or source 0)
                              :source_end (or last 10)}]})
(local base-rows (fcollect [i 1 40] (row (.. :row- i))))
(set misa.transcript_projection (fn [db] db.layout))
(local following {:messages {} :layout base-rows})
(local browsing (move following 15))
(assert (= (. (project browsing 10) :first) 16))
(fn insert-at [rows at]
  (local changed [])
  (each [i line (ipairs rows)]
    (when (= i at)
      (for [j 1 4] (table.insert changed (row (.. :inserted- j)))))
    (table.insert changed line))
  changed)
(each [_ scenario (ipairs [{:at 4 :first 20} {:at 19 :first 16} {:at 35 :first 16}])]
  (local updated (misa.patch browsing {:layout (misa.replace (insert-at base-rows scenario.at))}))
  (local view (project updated 10))
  (assert (= view.first scenario.first))
  (assert (= (. view.lines 1 :transcript_id) :row-16))
  (each [_ delta (ipairs [1 -1 3 -3 10 -10])]
    (local moved (move updated delta))
    (assert (= (. (project moved 10) :first) (- scenario.first delta))
            "streaming update swallowed or repeated a scroll delta"))
  ;; Removing those new rows must preserve the newly chosen content too.
  (local moved (move updated -2))
  (local top-id (. (. (project moved 10) :lines) 1 :transcript_id))
  (local contracted (misa.patch moved {:layout (misa.replace base-rows)}))
  (assert (= (. (. (project contracted 10) :lines) 1 :transcript_id) top-id)))
;; Following the tail remains a separate mode; scrolling back down restores it.
(local appended (insert-at base-rows 40))
(assert (= (. (project (misa.patch following {:layout (misa.replace appended)}) 10) :first) 35))
(local returned (move browsing -1000))
(assert (= returned.messages.top nil))
(assert (= returned.messages.anchor nil))
(assert (= (. (project (misa.patch returned {:layout (misa.replace appended)}) 10) :first) 35))
;; Content disappearing entirely cannot leave an invalid scroll position.
(local shortened (misa.patch browsing {:layout (misa.replace
                                  (icollect [i line (ipairs base-rows)] (when (<= i 20) line)))}))
(assert (= (. (project shortened 10) :first) 11) "shrinking content did not clamp the viewport")
(local moved-short (move shortened 3))
(assert (= (. (project moved-short 10) :first) 8) "scroll started from the stale position before contraction")
(assert (= (. (project (misa.patch moved-short {:layout (misa.replace base-rows)}) 10) :first) 8)
        "regrowth discarded the user's scroll after contraction")
(assert (= (. (project browsing 6) :first) 16) "editor growth moved the top visible content")
(assert (= (. (project following 6) :first) 35) "editor growth broke tail following")
(local empty-view (project (misa.patch browsing {:layout (misa.replace [])}) 10))
(assert (= empty-view.first 1))
(assert (= (length empty-view.lines) 0))
(set misa.transcript_projection original-projection)
;; A resize reveals the physical row containing the previously visible source
;; byte, even when the following row starts closer to that byte numerically.
(each [_ name (ipairs [:layout :markdown :component/markdown])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) {}))
(local source (table.concat (fcollect [i 1 80] (.. "word" i)) " "))
(set misa.transcript_projection
     (fn [_ context]
       (icollect [_ line (ipairs (misa.markdown_view.render (misa.markdown.parse source) {:columns context.columns}))]
         (misa.patch line {:transcript_id :long-block}))))
(local scrolled (scroll {:messages {}} {:delta 3} {:terminal {:lines 7 :columns 80}}))
(local anchored-db (misa.patch {:messages {}} scrolled.patch))
(local source-anchor anchored-db.messages.anchor.source)
(local resized (viewport anchored-db {:columns 31} 3))
(local visible (. resized.anchors resized.first))
(assert (<= visible.source source-anchor))
(assert (> visible.last source-anchor) "reflow skipped past the visible source byte")
(each [_ delta (ipairs [1 -1 3 -3])]
  (local result (scroll anchored-db {: delta} {:terminal {:lines 7 :columns 31}}))
  (local changed (misa.patch anchored-db result.patch))
  (assert (= (. (viewport changed {:columns 31} 3) :first) (- resized.first delta))
          "scroll concurrent with wrapping did not start from the relocated viewport"))
(local wider (viewport anchored-db {:columns 100} 3))
(local wide-visible (. wider.anchors wider.first))
(assert (<= wide-visible.source source-anchor) (fennel.view {:anchor source-anchor :visible wide-visible}))
(assert (> wide-visible.last source-anchor))
(assert (= (. (viewport {:messages {}} {:columns 31} 3) :first)
           (+ (- (length (misa.transcript_projection {} {:columns 31})) 3) 1)))
(set misa.transcript_projection original-projection)
(output "transcript viewport contracts passed\n")
