{:setup (fn []
          (misa.reg_event :app/start
                          (fn [db]
                            (var highlighted false)
                            (local native-highlight misa.syntax.highlight)

                            (fn misa.syntax.highlight [language source]
                              (set highlighted true)
                              (native-highlight language source))

                            (local source "# live
## second
***both*** and ~~**gone**~~ and `x` [docs](https://example.test)

- [ ] todo
  continuation
  2. nested

> quoted

---

| left | centered | right |
| :--- | :------: | ----: |
| a | growing value | 7 |

```bogus
return 42
")
                            (local document (misa.markdown.parse source))
                            (assert (and (and (= document.kind :document)
                                              (= (. document.blocks 1 :kind)
                                                 :heading))
                                         (= (. document.blocks 1 :level) 1)))
                            (local narrow
                                   (misa.markdown_view.render document
                                                              {:base :assistant
                                                               :columns 30}))
                            (local wide
                                   (misa.markdown_view.render document
                                                              {:base :assistant
                                                               :columns 60}))
                            (var (text narrow-top wide-top) (values "" nil nil))
                            (local saw
                                   {:both false
                                    :box false
                                    :code false
                                    :continuation false
                                    :h1 false
                                    :h2 false
                                    :link false
                                    :quote false
                                    :rule false
                                    :strike false
                                    :table false})
                            (each [_ line (ipairs narrow)]
                              (var line-text "")
                              (each [_ item (ipairs line.spans)]
                                (set line-text (.. line-text item.text))
                                (set text (.. text item.text))
                                (local styles {})
                                (when (= (type item.style) :table)
                                  (each [_ token (ipairs item.style)]
                                    (tset styles token true)))
                                (when (and (and (= item.text :both) styles.bold)
                                           styles.italic)
                                  (set saw.both true))
                                (when (and (and (= item.text :gone) styles.bold)
                                           styles.strikethrough)
                                  (set saw.strike true))
                                (when (and (= item.link "https://example.test")
                                           (= item.text :docs))
                                  (set saw.link true)))
                              (when (line-text:match "^█ ") (set saw.h1 true))
                              (when (line-text:match "^▌ ") (set saw.h2 true))
                              (when (line-text:find "☐ " 1 true)
                                (set saw.box true))
                              (when (line-text:match "^  continuation")
                                (set saw.continuation true))
                              (when (line-text:find "▏ " 1 true)
                                (set saw.quote true))
                              (when (= line-text (string.rep "─" 30))
                                (set saw.rule true))
                              (when (line-text:find "┌" 1 true)
                                (set saw.table true)
                                (set narrow-top (misa.layout.width line-text)))
                              (when (line-text:find :bogus 1 true)
                                (set saw.code true)))
                            (each [_ line (ipairs wide)]
                              (var line-text "")
                              (each [_ item (ipairs line.spans)]
                                (set line-text (.. line-text item.text)))
                              (when (line-text:find "┌" 1 true)
                                (set wide-top (misa.layout.width line-text))
                                (lua :break)))
                            (each [feature value (pairs saw)]
                              (assert value
                                      (.. "missing Markdown rendering: "
                                          feature)))
                            (assert highlighted
                                    "fenced code did not invoke the syntax service")
                            (assert (and (and (not (text:find "***" 1 true))
                                              (not (text:find "~~" 1 true)))
                                         (not (text:find "[docs]" 1 true)))
                                    "Markdown delimiters leaked")
                            (assert (and (and (and narrow-top wide-top)
                                              (<= narrow-top 30))
                                         (> wide-top narrow-top))
                                    "table columns did not respond to streaming width")
                            {: db
                             :fx [{:lines [{:spans [{:style {:foreground :default}
                                                     :text :markdown}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

