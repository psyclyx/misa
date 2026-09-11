(local definitions (require :tests.declarations))

;; The Lua layout measures text for wrapping and cursor mapping while the
;; presenter measures it natively for frame validation. They are separate
;; implementations of the same rules, so this walks a corpus that exercises the
;; rules and asserts that both answer identically. A divergence fails here
;; rather than producing a frame the presenter rejects.
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

(fn compare []
  "Return the first width or cell mismatch, or nil when both agree."
  (var problem nil)
  (each [_ codepoints (ipairs corpus) &until problem]
    (let [value (text codepoints)
          lua-width (misa.layout.width value)
          native-width (misa.native.width value)]
      (when (not= lua-width native-width)
        (set problem
             (.. "width " (tostring lua-width) " vs " (tostring native-width)
                 " for " (tostring value))))
      (each [_ codepoint (ipairs codepoints) &until problem]
        (let [lua-cells (misa.layout.cell-width codepoint)
              native-cells (misa.native.cell-width codepoint)]
          (when (not= lua-cells native-cells)
            (set problem
                 (.. "cell-width " (tostring lua-cells) " vs "
                     (tostring native-cells) " for U+"
                     (string.format "%X" codepoint))))))))
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
