(local definitions (require :tests.declarations))

;; Regression: ordinary text must not trigger a suffix search per byte.

(fn []
          (local declarations [])
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn []
                                    (fn equal [a b]
                                      (if (not= (type a) (type b))
                                          false
                                          (if (not= (type a) :table) (= a b)
                                              (do
                                                (each [k v (pairs a)]
                                                  (when (not (equal v (. b k)))
                                                    (lua "return false")))
                                                (each [k (pairs b)]
                                                  (when (= (. a k) nil)
                                                    (lua "return false")))
                                                true))))

                                    (local cases
                                           ["intro

# Heading

paragraph **bold** and [link](https://example.test) tail"
                                            "| a | b |
| --- | --- |
| x | y |

end"
                                            "intro\n| a | b |\n---|---\n| x | y |"
                                            "```lua\nprint(\"hi\")\n```\nnext\n\nend"
                                            "- item\n  continuation\n  more\n\nnext"
                                            "a\r\n\r\nb\r\n# c\r\ntail"
                                            "é界\n\n~~~\ncode\n~~~~\n\nlast"])
                                    ;; Deterministic malformed Markdown exercises reinterpretation of the last
                                    ;; line, not just neatly delimited tokens or complete provider chunks.
                                    (var seed 1591)
                                    (local alphabet "ab `[]()*_~\\\n#|>-\r")
                                    (for [_ 1 80]
                                      (local chars {})
                                      (for [i 1 80]
                                        (set seed (% (* seed 48271) 2147483647))
                                        (local n
                                               (+ (% seed (length alphabet)) 1))
                                        (tset chars i (alphabet:sub n n)))
                                      (tset cases (+ (length cases) 1)
                                            (table.concat chars)))
                                    (each [___case___ source (ipairs cases)]
                                      (local stream
                                             (misa.markdown.new-document))
                                      (for [size 0 (length source)]
                                        (local prefix (source:sub 1 size))
                                        (local actual (stream:update prefix))
                                        (assert (equal actual
                                                       (misa.markdown.parse prefix))
                                                (.. "streaming AST mismatch: "
                                                    ___case___ " byte " size))
                                        (assert (= (stream:update prefix)
                                                   actual)
                                                "unchanged source should retain its tree"))
                                      (assert (equal (stream:update :replacement)
                                                     (misa.markdown.parse :replacement)))
                                      (assert (equal (stream:update source)
                                                     (misa.markdown.parse source))))
                                    (local stream (misa.markdown.new-document))
                                    (local before (stream:update "# stable

paragraph

tail"))
                                    (local after (stream:update "# stable

paragraph

tail **bold**"))
                                    (assert (and (= (. before.blocks 1)
                                                    (. after.blocks 1))
                                                 (= (. before.blocks 3)
                                                    (. after.blocks 3)))
                                            "completed blocks should be reused")
                                    (assert (= (. before.blocks
                                                  (length before.blocks)
                                                  :inlines 1 :text)
                                               :tail)
                                            "updates must not mutate old trees")
                                    (local text (string.rep :x 300000))
                                    (local doc (misa.markdown.parse text))
                                    (local large (misa.markdown.new-document))
                                    (local first (large:update text))
                                    (local extended
                                           (large:update (.. text :more)))
                                    (assert (not= extended first)
                                            "appended source must extend the tree")
                                    (assert (= (. extended.blocks 1 :inlines 1
                                                  :text)
                                               (.. text :more)))
                                    (assert (= (. first.blocks 1 :inlines 1
                                                  :text)
                                               text)
                                            "appends mutated a previous snapshot")
                                    (assert (and (= doc.source text)
                                                 (= (length doc.blocks) 1)))
                                    (assert (= (. doc.blocks 1 :source_end)
                                               (length text)))
                                    (assert (= (. doc.blocks 1 :inlines 1 :text)
                                               text))
                                    (local many-blocks
                                           (misa.markdown.parse (.. (string.rep "# heading

"
                                                                                5000)
                                                                    :**BLOCK-TAIL**)))
                                    (assert (> (length many-blocks.blocks) 4096))
                                    (local last-block
                                           (. many-blocks.blocks
                                              (length many-blocks.blocks)))
                                    (assert (= (. last-block.inlines 1 :text)
                                               :BLOCK-TAIL))
                                    (assert (= (. last-block.inlines 1 :marks 1)
                                               :strong))
                                    (local many-inlines
                                           (misa.markdown.parse (.. (string.rep "**bold** "
                                                                                9000)
                                                                    :*INLINE-TAIL*)))
                                    (local nodes
                                           (. many-inlines.blocks 1 :inlines))
                                    (assert (> (length nodes) 16384))
                                    (assert (= (. nodes (length nodes) :text)
                                               :INLINE-TAIL))
                                    (assert (= (. nodes (length nodes) :marks 1)
                                               :emphasis))
                                    (local target
                                           (.. "https://example.test/"
                                               (string.rep :a 6000)))
                                    (local link
                                           (misa.markdown.parse (.. "[label]("
                                                                    target ")")))
                                    (assert (= (. link.blocks 1 :inlines 1
                                                  :target)
                                               target))
                                    (local deep
                                           (misa.markdown.parse (.. (string.rep "> "
                                                                                2000)
                                                                    :**deep**)))
                                    (assert (= (. deep.blocks 1 :depth) 2000))
                                    (assert (= (. deep.blocks 1 :inlines 1
                                                  :marks 1)
                                               :strong))
                                    (local list
                                           (misa.markdown.parse (.. (string.rep " "
                                                                                80)
                                                                    "- item")))
                                    (assert (= (. list.blocks 1 :depth) 41))
                                    (local language (string.rep :a 140))
                                    (assert (= (. (misa.markdown.parse (.. "```"
                                                                           language
                                                                           "
code
```")) :blocks 1 :language)
                                               language))
                                    (local brackets (string.rep "[" 8000))
                                    (assert (= (. (misa.markdown.parse brackets)
                                                  :blocks 1 :inlines 1 :text)
                                               brackets))
                                    (local mixed
                                           (misa.markdown.parse "prefix `code` and [label](https://example.test) and **bold** tail"))
                                    (local nodes (. mixed.blocks 1 :inlines))
                                    (assert (and (and (= (. nodes 1 :text)
                                                         "prefix ")
                                                      (= (. nodes 2 :kind)
                                                         :code))
                                                 (= (. nodes 2 :text) :code)))
                                    (assert (and (= (. nodes 4 :kind) :link)
                                                 (= (. nodes 4 :children 1
                                                       :text)
                                                    :label)))
                                    (assert (and (and (= (. nodes 6 :text)
                                                         :bold)
                                                      (= (. nodes 6 :marks 1)
                                                         :strong))
                                                 (= (. nodes 7 :text) " tail")))
                                    {:fx [{:lines [{:spans [{:text "markdown regressions"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
          nil
          (definitions.collect :tests.markdown declarations {}))
