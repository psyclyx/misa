;; Root layout contracts across short/tall terminals, wrapped inputs, docks,
;; and modal layers. Completion targets must match the rows users can see.
(local fennel (require :fennel))
(require :tests.application)
(fn lines [count label]
  (let [result []]
    (for [_ 1 count]
      (table.insert result {:spans [{:text label}]}))
    result))

(var view nil)
(var layers [])
(var inputs 0)
(var completion-count 0)
(var editor-layout nil)
(set _G.misa {:components {:render (fn [] {:lines (lines 2 :header)})}
              :status {:model (fn [] (lines 1 :status))}
              :ui {:layers (fn [] layers)}
              :editor {:layout (fn [_ context]
                                   (set editor-layout context.layout)
                                   {:input (lines inputs :input)
                                    :completions (lines completion-count
                                                        :completion)
                                    :row inputs
                                    :byte 0})}
              :transcript {:window (fn [_db _context count]
                                   (lines count :transcript))}})

(fn setup [extension]
  (local definitions (extension {}))
  (each [name value (pairs (or definitions.services {}))]
    (local parts (icollect [part (name:gmatch "[^.]+") ] part))
    (var target _G.misa)
    (for [index 1 (- (length parts) 1)]
      (local part (. parts index))
      (when (not (. target part)) (tset target part {}))
      (set target (. target part)))
    (tset target (. parts (length parts)) value))
  (each [_ render (pairs (or definitions.views {}))] (set view render)))

(setup (fennel.dofile :extensions/misa/ui/layout.fnl))
(setup (fennel.dofile :extensions/misa/ui/init.fnl))
(assert (> (_G.misa.ui.completion-room {} {:lines 48 :columns 80} 1) 9)
        "tall terminals should have room for more than nine completion candidates")

(for [height 1 60]
  (for [count 1 20]
    (for [dock 0 8]
      (set inputs count)
      (set completion-count 100)
      (set layers [{:dock :input :lines (lines dock :dock)}])
      (let [terminal {:lines height :columns 80}
            room (_G.misa.ui.completion-room {} terminal count)
            frame (view {} {: terminal})]
        (assert (<= (length frame.lines) height) "frame exceeds viewport")
        (assert (<= 1 frame.cursor.row (length frame.lines))
                "input cursor exceeds frame")
        (var completions 0)
        (each [_ line (ipairs frame.lines)]
          (when (= (. line.spans 1 :text) :completion)
            (set completions (+ completions 1))))
        (assert (= completions room)
                "positional keys disagree with visible completion rows")
        (assert (= editor-layout.dock_count dock))
        (assert (= (. (_G.misa.ui.input-budgets editor-layout count dock) :completions) room)
                "editor did not receive the root layout budget")))))

(for [height 1 30]
  (for [count 0 40]
    (each [_ kind (ipairs [:overlay :exclusive])]
      (set layers
           [{kind true
             :lines (lines count :layer)
             :cursor (when (> count 0) {:row count :byte 0 :shape :block})}])
      (let [frame (view {} {:terminal {:lines height :columns 80}})]
        (assert (<= (length frame.lines) height) "layer exceeds viewport")
        (when frame.cursor
          (assert (= frame.cursor.shape :block) "composition lost cursor shape")
          (assert (<= 1 frame.cursor.row (length frame.lines))
                  "layer cursor exceeds frame"))))))

(assert (= (length (. (view {} {:terminal {:lines 0 :columns 80}}) :lines)) 0)
        "zero-height terminal emitted content")

(let [animation {:id :clip :interval_ms 100 :frames [{:text :xx} {:text :yy}]}
      source [{:spans [{:text :xx : animation}]}]
      full (_G.misa.ui.bound-frame source 2)
      clipped (_G.misa.ui.bound-frame source 1)]
  (assert (= (. full.lines 1 :spans 1 :animation) animation)
          "root clipping lost a fully visible animation")
  (assert (= (. clipped.lines 1 :spans 1 :text) :x))
  (assert (= (. clipped.lines 1 :spans 1 :animation) nil)
          "root clipping retained frames wider than their fallback")
  (assert (= (. source 1 :spans 1 :animation) animation)
          "root clipping mutated the source animation"))

(each [id component (pairs (. ((fennel.dofile :extensions/misa/editor/render.fnl) {}) :components))]
  (when (= id :default.editor.input)
    (each [_ mode (ipairs [:insert :normal :visual])]
      (local input (component.render {:text "hello" :cursor 2 : mode} {:columns 20}))
      (assert (= input.cursor.shape (if (= mode :insert) :bar :block))
              "editor mode must choose the native cursor shape"))))

(print "layout contracts passed")
