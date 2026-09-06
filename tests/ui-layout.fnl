;; Root layout contracts across short/tall terminals, wrapped inputs, docks,
;; and modal layers. Completion targets must match the rows users can see.
(local fennel (require :fennel))
(fn lines [count label]
  (let [result []]
    (for [_ 1 count]
      (table.insert result {:spans [{:text label}]}))
    result))

(var view nil)
(var layers [])
(var inputs 0)
(var completion-count 0)
(set _G.misa {:render_component (fn [] {:lines (lines 2 :header)})
              :status_projection (fn [] (lines 1 :status))
              :view_layers (fn [] layers)
              :editor_projection (fn []
                                   {:input (lines inputs :input)
                                    :completions (lines completion-count
                                                        :completion)
                                    :row inputs
                                    :byte 0})
              :transcript_window (fn [_db _context count]
                                   (lines count :transcript))})

(fn setup [extension]
  (each [_ effect (ipairs (. (extension.setup) :fx))]
    (match effect.type
      :register/service (tset _G.misa effect.name effect.value)
      :register/view (set view effect.handler)
      _ (error (.. "unexpected layout setup effect: " effect.type)))))

(setup (fennel.dofile :extensions/layout.fnl))
(setup (fennel.dofile :extensions/ui.fnl))
(assert (> (_G.misa.inline_choice_room {} {:lines 48 :columns 80} 1) 9)
        "tall terminals should have room for more than nine completion candidates")

(for [height 1 60]
  (for [count 1 20]
    (for [dock 0 8]
      (set inputs count)
      (set completion-count 100)
      (set layers [{:dock :input :lines (lines dock :dock)}])
      (let [terminal {:lines height :columns 80}
            room (_G.misa.inline_choice_room {} terminal count)
            frame (view {} {: terminal})]
        (assert (<= (length frame.lines) height) "frame exceeds viewport")
        (assert (<= 1 frame.cursor.row (length frame.lines))
                "input cursor exceeds frame")
        (var completions 0)
        (each [_ line (ipairs frame.lines)]
          (when (= (. line.spans 1 :text) :completion)
            (set completions (+ completions 1))))
        (assert (= completions room)
                "positional keys disagree with visible completion rows")))))

(for [height 1 30]
  (for [count 0 40]
    (each [_ kind (ipairs [:overlay :exclusive])]
      (set layers
           [{kind true
             :lines (lines count :layer)
             :cursor (when (> count 0) {:row count :byte 0})}])
      (let [frame (view {} {:terminal {:lines height :columns 80}})]
        (assert (<= (length frame.lines) height) "layer exceeds viewport")
        (when frame.cursor
          (assert (<= 1 frame.cursor.row (length frame.lines))
                  "layer cursor exceeds frame"))))))

(assert (= (length (. (view {} {:terminal {:lines 0 :columns 80}}) :lines)) 0)
        "zero-height terminal emitted content")

(let [animation {:id :clip :interval_ms 100 :frames [{:text :xx} {:text :yy}]}
      source [{:spans [{:text :xx : animation}]}]
      full (_G.misa.ui_bound_frame source 2)
      clipped (_G.misa.ui_bound_frame source 1)]
  (assert (= (. full.lines 1 :spans 1 :animation) animation)
          "root clipping lost a fully visible animation")
  (assert (= (. clipped.lines 1 :spans 1 :text) :x))
  (assert (= (. clipped.lines 1 :spans 1 :animation) nil)
          "root clipping retained frames wider than their fallback")
  (assert (= (. source 1 :spans 1 :animation) animation)
          "root clipping mutated the source animation"))

(print "layout contracts passed")
