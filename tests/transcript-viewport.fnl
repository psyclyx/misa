;; Root layout contracts across short/tall terminals, wrapped inputs, docks,
;; and modal layers. Completion targets must match the rows users can see.
(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :tests.declarations))
(app.define (. (require :tests.stock) :misa.json))
(local specs (. (require :tests.stock) :misa.transcript))
(var (viewport scroll) nil)
(set viewport (. specs.services :transcript.viewport))
(set scroll (. specs.events :messages/messages/scroll :handler))
(each [_ name (ipairs [:misa.ui.layout :misa.markdown :misa.markdown.render])]
  (app.define (. (require :tests.stock) name)))
(app.install)
(set misa.transcript {})
(set misa.selection {})
(var measured 0)

;; The transcript presents measured geometry: items in order, each with its rows
;; and the row its first row occupies. A window materializes rows from that and
;; never walks the transcript, so these contracts drive geometry directly.
(fn layout-of [lines selected-row]
  (let [items []
        offsets {}]
    (var row 1)
    (each [index line (ipairs lines)]
      (tset offsets index row)
      (table.insert items
                    {:id line.transcript_id
                     :role :transcript.user
                     :model {}
                     :entry {:view {:lines [line]}}
                     :lines [line]
                     :height 1})
      (set row (+ row 1)))
    {:items items
     :offsets offsets
     :total (- row 1)
     :context {}
     :selected_row selected-row}))

(fn rows-of [db]
  (icollect [index (ipairs (or db.rows []))]
    {:transcript_id (tostring index) :spans [{:text (tostring index)}]}))

(set misa.transcript.layout
     (fn [db _context]
       (set measured (+ measured 1))
       (layout-of (rows-of db) db.selected_row)))
(set misa.transcript.rows
     (fn [_db layout first count]
       (icollect [index (ipairs layout.items)]
         (when (and (>= index first) (< index (+ first count)))
           (. (. layout.items index) :lines 1)))))
(set misa.selection.state (fn [db] db.selected))

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
(set misa.ui.regions (fn [db terminal]
                       [{:id :transcript
                         :viewport (viewport db {:columns terminal.columns}
                                             (- terminal.lines 4))}]))
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
(assert (= (. (project (misa.patch tail {:rows (misa.replace (. (snapshot 150) :rows))}) 10) :first)
           86))
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
(set measured 0)
(project original 10)
(assert (= measured 1) "viewport measured the transcript more than once")
(set measured 0)
(move original 3)
(assert (= measured 1) "scroll measured the transcript more than once")
(local original-layout misa.transcript.layout)

;; A focused selection is revealed from measured geometry rather than from
;; painted flags, so a row outside the window still moves into it.
(set misa.transcript.layout
     (fn [db context]
       (local layout (original-layout db context))
       (set layout.selected_row (or db.selected_row layout.selected_row))
       layout))
(assert (= (. (project selected 10) :first) 12)
        "the focused selection was not revealed")
(assert (= (. (project (snapshot 100 5) 10) :first) 5)
        "a selection above the window was not scrolled into view")
(set misa.transcript.layout original-layout)
;; Width/detail changes alter row counts before the anchor, not its identity.
(set misa.transcript.layout
     (fn [_db context]
       (let [per-block (if (= context.columns 40) 4 2)
             rows []]
         (for [block 1 30]
           (for [row 1 per-block]
             (table.insert rows
                           {:transcript_id (tostring block)
                            :source_start 0
                            :spans [{:text (.. block ":" row)}]})))
         (layout-of rows nil))))
(local anchored (move original 15))
(local before-resize (viewport anchored {:columns 80} 10))
(local after-resize (viewport anchored {:columns 40} 10))
(assert (= (. before-resize.lines 1 :transcript_id) (. after-resize.lines 1 :transcript_id))
        "resize lost the scrolled content block")
(local followed (viewport original {:columns 40} 10))
(assert (= followed.first 111) "tail-following acquired a content anchor")
(set misa.transcript.layout original-layout)

;; Streaming and scroll events share the same rule: preserve the visible
;; content first, then apply the requested row delta exactly once.
(fn row [id source last]
  {:transcript_id id
   :spans [{:text id :source true :source_start (or source 0)
            :source_end (or last 10)}]})
(local base-rows (fcollect [i 1 40] (row (.. :row- i))))
(set misa.transcript.layout (fn [db] (layout-of db.layout nil)))
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
(assert (= (. (project (misa.patch following {:layout (misa.replace appended)}) 10) :first)
           35))
(local returned (move browsing -1000))
(assert (= returned.messages.top nil))
(assert (= returned.messages.anchor nil))
(assert (= (. (project (misa.patch returned {:layout (misa.replace appended)}) 10) :first)
           35))
;; Content disappearing entirely cannot leave an invalid scroll position.
(local shortened (misa.patch browsing {:layout (misa.replace
                                                  (icollect [i line (ipairs base-rows)]
                                                    (when (<= i 20) line)))}))
(assert (= (. (project shortened 10) :first) 11) "shrinking content did not clamp the viewport")
(local moved-short (move shortened 3))
(assert (= (. (project moved-short 10) :first) 8) "scroll started from the stale position before contraction")
(assert (= (. (project (misa.patch moved-short {:layout (misa.replace base-rows)}) 10) :first)
           8)
        "regrowth discarded the user's scroll after contraction")
(assert (= (. (project browsing 6) :first) 16) "editor growth moved the top visible content")
(assert (= (. (project following 6) :first) 35) "editor growth broke tail following")
(local empty-view (project (misa.patch browsing {:layout (misa.replace [])}) 10))
(assert (= empty-view.first 1))
(assert (= (length empty-view.lines) 0))
(set misa.transcript.layout original-layout)
;; A resize reveals the physical row containing the previously visible source
;; byte, even when the following row starts closer to that byte numerically.

(local source (table.concat (fcollect [i 1 80] (.. "word" i)) " "))
(set misa.transcript.layout
     (fn [_ context]
       (layout-of (icollect [_ line (ipairs (misa.markdown.view.render (misa.markdown.parse source)
                                                                       {:columns context.columns}))]
                    (misa.patch line {:transcript_id :long-block}))
                  nil)))
(local scrolled (scroll {:messages {}} {:delta 3} {:terminal {:lines 7 :columns 80}}))
(local anchored-db (misa.patch {:messages {}} scrolled.patch))
(local source-anchor anchored-db.messages.anchor.source)
(local resized (viewport anchored-db {:columns 31} 3))
(local visible resized.anchor)
(assert (<= visible.source source-anchor))
(assert (> visible.last source-anchor) "reflow skipped past the visible source byte")
(each [_ delta (ipairs [1 -1 3 -3])]
  (local result (scroll anchored-db {: delta} {:terminal {:lines 7 :columns 31}}))
  (local changed (misa.patch anchored-db result.patch))
  (assert (= (. (viewport changed {:columns 31} 3) :first) (- resized.first delta))
          "scroll concurrent with wrapping did not start from the relocated viewport"))
(local wider (viewport anchored-db {:columns 100} 3))
(local wide-visible wider.anchor)
(assert (<= wide-visible.source source-anchor)
        (fennel.view {:anchor source-anchor :visible wide-visible}))
(assert (> wide-visible.last source-anchor))
(let [layout (misa.transcript.layout {:messages {}} {:columns 31})]
  (assert (= (. (viewport {:messages {}} {:columns 31} 3) :first)
             (+ (- layout.total 3) 1))))
(set misa.transcript.layout original-layout)

(output "transcript viewport contracts passed\n")
