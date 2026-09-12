(local definitions (require :tests.declarations))

;; `misa.ui.layout` measures text in Lua for wrapping and cursor mapping while the
;; presenter measures it natively for frame validation. The layout measures
;; through `misa.native` now, so this compares the two places a measurement can
;; come from: the native module itself and the offline stand-in the standalone
;; harness loads in its place. Every codepoint is checked, and the grapheme
;; corpus below covers the rules a single codepoint cannot.

(local offline (require :tests.native-layout))

(fn utf8 [codepoint]
  (if (<= codepoint 127) (string.char codepoint) (<= codepoint 2047)
      (string.char (+ 192 (math.floor (/ codepoint 64)))
                   (+ 128 (% codepoint 64))) (<= codepoint 65535)
      (string.char (+ 224 (math.floor (/ codepoint 4096)))
                   (+ 128 (% (math.floor (/ codepoint 64)) 64))
                   (+ 128 (% codepoint 64)))
      (string.char (+ 240 (math.floor (/ codepoint 262144)))
                   (+ 128 (% (math.floor (/ codepoint 4096)) 64))
                   (+ 128 (% (math.floor (/ codepoint 64)) 64))
                   (+ 128 (% codepoint 64)))))

(fn text [codepoints]
  (table.concat (icollect [_ codepoint (ipairs codepoints)] (utf8 codepoint))))

;; ASCII, wide CJK, an emoji ZWJ sequence, a regional-indicator flag, a keycap
;; sequence, a combining mark, a virama-joined cluster, a variation selector, a
;; Hangul jamo, a zero-width space, a skin-tone modifier, and a mixture.
(local corpus [[104 101 108 108 111]
               [20320 22909]
               [128105 8205 128187]
               [127482 127480]
               [49 65039 8419]
               [101 769]
               [2325 2381 2359]
               [10084 65039]
               [4352]
               [8203]
               [128077 127997]
               [97 20320 128512 98]])

(fn same-clusters? [a b]
  (if (not= (length a) (length b))
      false
      (let [clusters (length a)]
        (var (index same?) (values 1 true))
        (while (and same? (<= index clusters))
          (set same? (= (. a index) (. b index)))
          (set index (+ index 1)))
        same?)))

(fn compare []
  "Return the first measurement mismatch, or nil when every rule agrees."
  (var problem nil)
  (for [codepoint 0 1114111 &until problem]
    (let [stand-in (offline.cell-width codepoint)
          native (misa.native.cell-width codepoint)]
      (when (not= stand-in native)
        (set problem (.. "cell-width " (tostring stand-in) " vs "
                         (tostring native) " for U+"
                         (string.format "%X" codepoint))))))
  (each [_ codepoints (ipairs corpus) &until problem]
    (let [value (text codepoints)
          stand-in (offline.width value)
          lua-width (misa.layout.width value)
          native-width (misa.native.width value)]
      (when (not= lua-width native-width)
        (set problem
             (.. "width " (tostring lua-width) " vs " (tostring native-width)
                 " for " (tostring value))))
      (when (not= stand-in native-width)
        (set problem
             (.. "offline width " (tostring stand-in) " vs "
                 (tostring native-width) " for " (tostring value))))
      (when (not (same-clusters? (offline.clusters value)
                                 (misa.native.clusters value)))
        (set problem (.. "clusters differ for " (tostring value))))))
  problem)

(fn [_context]
  (definitions.collect :tests.integration.fixtures.layout-parity
    [{:catalog :events
      :value {:event :app/start
              :handler (fn [_ _]
                         (let [problem (compare)]
                           {:fx [{:lines [{:spans [{:text (if problem
                                                              (.. "layout mismatch: "
                                                                  problem)
                                                              "layout parity ok")}]}]
                                  :type :view/commit}
                                 {:type :app/quit}]}))}}]
    {}))
