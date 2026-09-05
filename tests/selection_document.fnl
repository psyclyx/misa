{:setup (fn []
          (misa.reg_event :app/start
                          (fn [db]
                            (fn source [doc item]
                              (doc.text:sub (+ item.first 1) item.last))

                            (local text "# First\r
\r
A é paragraph\r
## Empty\r
# Next\r
Tail\r
")
                            (local doc
                                   (misa.selection_document :one :Message text))
                            (assert (and (= doc.text text)
                                         (= (source doc doc) text))
                                    "copy changed original newlines")
                            (local first (. doc.children 1))
                            (assert (= (source doc (. first.children 1))
                                       "# First")
                                    "heading offset mapped into CRLF")
                            (local content (. first.children 2))
                            (assert (= (source doc (. content.children 1))
                                       "A é paragraph")
                                    "paragraph range not mapped from normalized bytes")
                            (local empty (. content.children 2))
                            (assert (= (. empty.children 2 :first)
                                       (. empty.children 2 :last))
                                    "empty section content has invalid range")
                            (local table-text "| a | b | c |\r
| --- | --- | --- |\r
| --- | keep | ``a|`b`` |\r
|  | é | 👩‍💻 |")
                            (local table-doc
                                   (misa.selection_document :table :Table
                                                            table-text))
                            (local rows (. table-doc.children 1 :children))
                            (assert (and (= (length rows) 3)
                                         (= (length (. rows 2 :children)) 3))
                                    "table parser dropped data row or split code span")
                            (assert (= (source table-doc (. rows 2 :children 3))
                                       "``a|`b``")
                                    "code cell range lost original syntax")
                            (assert (and (= (length (. rows 3 :children)) 3)
                                         (= (source table-doc
                                                    (. rows 3 :children 1))
                                            ""))
                                    "empty cell disappeared")
                            (each [_ index (ipairs [2 3])]
                              (local word
                                     (. (misa.selection_children table-doc
                                                                 (. rows 3
                                                                    :children
                                                                    index))
                                        1))
                              (local chars
                                     (misa.selection_children table-doc word))
                              (assert (= (length chars) 1)
                                      "fine selection split a grapheme"))
                            (local large (string.rep :x 3000))
                            (local large-doc
                                   (misa.selection_document :large :Large large))
                            (assert (and (= large-doc.text large)
                                         (= large-doc.last (length large)))
                                    "selection lost source text")
                            (local last
                                   (. large-doc.children
                                      (length large-doc.children)))
                            (assert (= last.last (length large))
                                    "source tail is unreachable")
                            (for [height 0 12]
                              (local frame
                                     (misa.render_component db :selection {:hints {}
                                                                           :index 1
                                                                           :nodes [{:label :Message}]
                                                                           :path [:Message]
                                                                           :text "a\tb\r
b
c"}
                                                            {:available_lines height
                                                             :columns 30}))
                              (assert (<= (length frame.lines) height)
                                      "selection exceeds viewport")
                              (each [_ row (ipairs frame.lines)]
                                (each [_ part (ipairs row.spans)]
                                  (assert (not (part.text:find "[\t\r\n]"))
                                          "preview has control newline inside physical row"))))
                            {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                     :text "selection document"}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

