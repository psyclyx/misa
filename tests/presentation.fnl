;; Projection contracts: surfaces survive rich spans, and popups retain context.

{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
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

                                    (fn line-text [line]
                                      (local parts {})
                                      (each [_ item (ipairs (or line.spans {}))]
                                        (tset parts (+ (length parts) 1)
                                              item.text))
                                      (table.concat parts))

                                    (local words
                                           (misa.layout.flow_spans [{:action :test
                                                                     :style :plain
                                                                     :text "alpha be"}
                                                                    {:link "https://example.test"
                                                                     :style :bold
                                                                     :text "ta gamma"}]
                                                                   8))
                                    (assert (and (and (and (= (length words) 3)
                                                           (= (line-text (. words
                                                                            1))
                                                              :alpha))
                                                      (= (line-text (. words 2))
                                                         :beta))
                                                 (= (line-text (. words 3))
                                                    :gamma))
                                            "word wrapping follows style boundaries or breaks ordinary words")
                                    (assert (and (= (. words 2 :spans 1 :action)
                                                    :test)
                                                 (= (. words 2 :spans 2 :link)
                                                    "https://example.test"))
                                            "word wrapping discarded semantic metadata")
                                    (local explicit
                                           (misa.layout.flow_spans [{:style :plain
                                                                     :text "a\n\nb"}]
                                                                   8))
                                    (assert (and (= (length explicit) 3)
                                                 (= (line-text (. explicit 2))
                                                    ""))
                                            "word wrapping lost explicit empty lines")
                                    (local long
                                           (.. (string.rep :a 10000) "é界"))
                                    (local long-lines
                                           (misa.layout.flow_spans [{:style :plain
                                                                     :text long}]
                                                                   7))
                                    (local pieces {})
                                    (each [_ line (ipairs long-lines)]
                                      (local text (line-text line))
                                      (assert (<= (misa.layout.width text) 7)
                                              "long token escaped its width")
                                      (tset pieces (+ (length pieces) 1) text))
                                    (assert (= (table.concat pieces) long)
                                            "long-token fallback split a grapheme or lost text")
                                    (local input-words
                                           (misa.layout.wrap_input "alpha beta"
                                                                   10 6 "│ "
                                                                   :plain
                                                                   :accent))
                                    (assert (and (= (. input-words.lines 1
                                                       :spans 2 :text)
                                                    "alpha ")
                                                 (= (. input-words.lines 2
                                                       :spans 2 :text)
                                                    :beta))
                                            "editor did not preserve whitespace while wrapping words")
                                    (assert (and (= input-words.cursor.row 2)
                                                 (= input-words.cursor.byte
                                                    (length (. input-words.lines
                                                               2 :spans 1 :text))))
                                            "editor word-wrap cursor did not follow its source byte")
                                    (local palette (. (misa.theme db) :palette))
                                    (assert (and (and (= palette.text :default)
                                                      (= (type palette.accent)
                                                         :table))
                                                 (= (type (. palette :function))
                                                    :table))
                                            "default theme did not inherit text and use truecolor accents")
                                    (local stream
                                           (misa.markdown_view.new_document))
                                    (local source
                                           "# Heading

```lua
print('hello')
```

| a | b |
| --- | --- |
| **bold** | 界 |

last [link](https://example.test)")
                                    (for [size 0 (length source) 3]
                                      (local text (source:sub 1 size))
                                      (each [_ columns (ipairs [12 40])]
                                        (local options
                                               {:base :assistant : columns})
                                        (local lines
                                               (stream:render text options))
                                        (assert (equal lines
                                                       (misa.markdown_view.render (misa.markdown.parse text)
                                                                                  options))
                                                "incremental layout differs from full layout")
                                        (assert (= (stream:render text options)
                                                   lines)
                                                "unchanged redraw recomputed Markdown layout")))
                                    (local options
                                           {:base [:assistant :dim]
                                            :columns 40})
                                    (assert (equal (stream:render source
                                                                  options)
                                                   (misa.markdown_view.render (misa.markdown.parse source)
                                                                              options))
                                            "style change left stale layout")
                                    (local model-cached
                                           {:id :cached
                                            :rail :rail.assistant
                                            :response_id :response
                                            :text source})
                                    (local context-cached
                                           {:columns 40 :interactive true})
                                    (local first
                                           (misa.render_component db
                                                                  :transcript.assistant
                                                                  model-cached
                                                                  context-cached))
                                    (assert (equal first
                                                   (misa.render_component db
                                                                          :transcript.assistant
                                                                          model-cached
                                                                          context-cached))
                                            "theme resolution mutated cached spans")
                                    (local rendered
                                           (misa.render_component db
                                                                  :transcript.assistant
                                                                  {:rail :rail.assistant
                                                                   :text "A **bold** [link](https://example.test)"}
                                                                  {:columns 40
                                                                   :interactive true}))
                                    (var link false)
                                    (each [_ line (ipairs rendered.lines)]
                                      (var text "")
                                      (each [_ item (ipairs line.spans)]
                                        (set text (.. text item.text))
                                        (assert item.style.background
                                                "message surface lost at a style boundary")
                                        (when item.link
                                          (set link
                                               (= item.link
                                                  "https://example.test")))
                                        (when (= item.text :bold)
                                          (assert (and (= item.style.foreground
                                                          :default)
                                                       item.style.bold)
                                                  "body color did not inherit terminal text")))
                                      (assert (= (misa.layout.width text) 40)
                                              "message surface did not fill its row"))
                                    (each [_ line (ipairs (misa.markdown_view.render (misa.markdown.parse "```lua
local x = 1
```")
                                                                                     {:columns 32}))]
                                      (local text (line-text line))
                                      (assert (and (not (text:find "╭" 1 true))
                                                   (not (text:find "╰" 1 true)))
                                              "code chrome retained curved corners")
                                      (assert (and (not= line.source_start nil)
                                                   (not= line.source_end nil))
                                              "Markdown layout discarded source block ranges"))
                                    (assert link
                                            "message URL lost link metadata")
                                    ;; Deep semantic nesting must leave room for the full body.
                                    (each [_ kind (ipairs [:quote
                                                           :list_item
                                                           :list_continuation])]
                                      (each [_ columns (ipairs [2 3 8 24])]
                                        (local body "界abcdefgh界")
                                        (local pieces {})
                                        (local block
                                               {: kind
                                                :depth 100000
                                                :inlines [{:kind :text
                                                           :text body}]
                                                :source_start 0
                                                :source_end (length body)})
                                        (each [_ line (ipairs (misa.markdown_view.render {:blocks [block]}
                                                                                         {: columns}))]
                                          (assert (<= (misa.layout.width (line-text line))
                                                      columns)
                                                  (.. "nested Markdown prefix escaped the viewport: "
                                                      kind " columns " columns
                                                      " text " (line-text line)))
                                          (each [_ item (ipairs line.spans)]
                                            (when item.source
                                              (tset pieces
                                                    (+ (length pieces) 1)
                                                    item.text))))
                                        (assert (= (table.concat pieces) body)
                                                "nested Markdown lost body text")
                                        (assert (= block.depth 100000)
                                                "layout changed semantic nesting depth")))
                                    ;; Captures and plain gaps may both cross line boundaries.
                                    (local code-source "alpha\nbeta\n\ngamma\n")
                                    (local code-lines
                                           (misa.markdown_view.render {:blocks [{:kind :code_block
                                                                                 :language :test
                                                                                 :text code-source
                                                                                 :source_start 0
                                                                                 :source_end (length code-source)}]}
                                                                      {:columns 32
                                                                       :captures {0 [{:start_byte 0
                                                                                      :end_byte 10
                                                                                      :capture :keyword}
                                                                                     {:start_byte 12
                                                                                      :end_byte 18
                                                                                      :capture :string}]}}))
                                    (local code-pieces {})
                                    (for [index 2 (- (length code-lines) 1)]
                                      (local pieces {})
                                      (each [_ item (ipairs (. code-lines index
                                                               :spans))]
                                        (when item.source
                                          (tset pieces (+ (length pieces) 1)
                                                item.text)))
                                      (tset code-pieces
                                            (+ (length code-pieces) 1)
                                            (table.concat pieces)))
                                    (assert (= (table.concat code-pieces "\n")
                                               code-source)
                                            "highlighted code lost a line or trailing newline")
                                    (assert (= (. code-lines 2 :spans 2 :style
                                                  3)
                                               :syntax.keyword)
                                            "highlighted code lost capture styling")
                                    (local long-language
                                           (string.rep :language 20))
                                    (local long-language-lines
                                           (misa.markdown_view.render {:blocks [{:kind :code_block
                                                                                 :language long-language
                                                                                 :text "retained body"
                                                                                 :source_start 0
                                                                                 :source_end 13}]}
                                                                      {:columns 32}))
                                    (assert (= (. long-language-lines 2 :spans
                                                  2 :text)
                                               "retained body")
                                            "unsupported language identifier discarded code")
                                    (assert (= (. long-language-lines 2 :spans
                                                  2 :style 2)
                                               :code)
                                            "unsupported language identifier lost code styling")
                                    (local table-source
                                           "| Key | Value |
| --- | --- |
| **styled** tail | 界界 more |")
                                    (each [_ columns (ipairs [80 24 16 10 8])]
                                      (local table-view
                                             (misa.render_component db
                                                                    :transcript.assistant
                                                                    {:rail :rail.assistant
                                                                     :text table-source}
                                                                    {: columns
                                                                     :interactive true}))
                                      (var (top bottom) (values false false))
                                      (each [_ line (ipairs table-view.lines)]
                                        (var text "")
                                        (each [_ part (ipairs line.spans)]
                                          (set text (.. text part.text)))
                                        (assert (= (misa.layout.width text)
                                                   columns)
                                                "table content escaped terminal width")
                                        (set text (text:gsub "%s+$" ""))
                                        (when (text:find "┌" 1 true)
                                          (set top true)
                                          (assert (= (text:sub (- (length "┐")))
                                                     "┐")
                                                  "table lost top right corner"))
                                        (when (text:find "└" 1 true)
                                          (set bottom true)
                                          (assert (= (text:sub (- (length "┘")))
                                                     "┘")
                                                  "table lost bottom right corner"))
                                        (when (text:find "┃ │" 1 true)
                                          (assert (= (text:sub (- (length "│")))
                                                     "│")
                                                  "wrapped table cell lost right border")))
                                      (when (>= columns 16)
                                        (assert (and top bottom)
                                                "bordered table unexpectedly disappeared")))
                                    (assert (and (= (misa.keybinding_text :g)
                                                    :g)
                                                 (= (misa.keybinding_text :G)
                                                    :G))
                                            "Vim hints collapse case-sensitive motions")
                                    (assert (and (= (misa.keybinding_text :alt+g)
                                                    "⌥G")
                                                 (= (misa.keybinding_text :f1)
                                                    :F1))
                                            "chord/function hints lost display convention")
                                    (local model
                                           {:cancellable true
                                            :code :1234
                                            :input "a long paste across lines"
                                            :input_enabled true
                                            :message "Authorize this device"
                                            :title :Login
                                            :url "https://example.test/login"})
                                    (for [height 0 12]
                                      (local popup
                                             (misa.render_component db :dialog
                                                                    model
                                                                    {:available_lines height
                                                                     :columns 24}))
                                      (assert (and popup.overlay
                                                   (not popup.exclusive))
                                              "dialog hides the transcript")
                                      (assert (<= (length popup.lines) height)
                                              "popup exceeds available height")
                                      (when popup.cursor
                                        (assert (and (>= popup.cursor.row 1)
                                                     (<= popup.cursor.row
                                                         (length popup.lines)))
                                                "popup cursor escaped viewport")))
                                    (local popup
                                           (misa.render_component db :dialog
                                                                  model
                                                                  {:available_lines 12
                                                                   :columns 80}))
                                    (var linked false)
                                    (each [_ line (ipairs popup.lines)]
                                      (each [_ item (ipairs line.spans)]
                                        (when (= item.link model.url)
                                          (set linked true))))
                                    (assert linked "login URL is not clickable")
                                    (local selected
                                           (misa.render_component db
                                                                  :editor.input
                                                                  {:cursor 4
                                                                   :mode :visual
                                                                   :selection_end 6
                                                                   :selection_start 1
                                                                   :text "a界b
c"}
                                                                  {:columns 6}))
                                    (var content "")
                                    (each [_ line (ipairs selected.lines)]
                                      (each [index item (ipairs line.spans)]
                                        (when (and (> index 1)
                                                   item.style.underline)
                                          (set content (.. content item.text)))))
                                    (assert (= content "界b")
                                            "visual selection split a grapheme or selected beyond its range")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :presentation}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
