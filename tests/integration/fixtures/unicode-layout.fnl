(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn [db]
                                    (assert (= (misa.layout.width "é界😀")
                                               5)
                                            "cell width disagrees with terminal semantics")
                                    (local wrapped
                                           (misa.layout.wrap-spans [{:spans [{:style :plain
                                                                              :text "é界x"}]}]
                                                                   3))
                                    (assert (and (and (= (length wrapped) 2)
                                                      (= (. wrapped 1 :spans 1
                                                            :text)
                                                         "é界"))
                                                 (= (. wrapped 2 :spans 1 :text)
                                                    :x))
                                            "Unicode span wrapping split or mismeasured text")
                                    (assert (= (misa.layout.width (misa.layout.fit "界"
                                                                                   4))
                                               4)
                                            "cell fitting did not pad by cells")
                                    (assert (and (= (misa.layout.previous-boundary "éx"
                                                                                   3)
                                                    0)
                                                 (= (misa.layout.next-boundary "éx"
                                                                               0)
                                                    3))
                                            "combining grapheme boundaries diverged")
                                    (assert (and (= (misa.layout.previous-boundary "👩‍💻x"
                                                                                   11)
                                                    0)
                                                 (= (misa.layout.next-boundary "👩‍💻x"
                                                                               0)
                                                    11))
                                            "ZWJ grapheme boundaries diverged")
                                    (assert (and (= (misa.layout.previous-boundary "क्x"
                                                                                   6)
                                                    0)
                                                 (= (misa.layout.next-boundary "क्x"
                                                                               0)
                                                    6))
                                            "virama grapheme boundaries diverged")
                                    (assert (and (= (misa.layout.previous-boundary "क्षx"
                                                                                   9)
                                                    0)
                                                 (= (misa.layout.next-boundary "क्षx"
                                                                               0)
                                                    9))
                                            "Devanagari conjunct boundaries diverged")
                                    (local input
                                           (misa.components.render db
                                                                   :editor.input
                                                                   {:cursor 5
                                                                    :text "ab界é👩‍💻क्ष
z"}
                                                                   {:columns 6}))
                                    (assert (and (= (length input.lines) 3)
                                                 (= (. input.lines 1 :spans 2
                                                       :text)
                                                    "ab界"))
                                            "input did not wrap on cell/grapheme boundaries")
                                    (assert (and (= input.cursor.row 2)
                                                 (= input.cursor.byte
                                                    (length (. input.lines 2
                                                               :spans 1 :text))))
                                            "logical cursor did not map across a prompt-prefixed soft wrap")
                                    (local narrow
                                           (misa.components.render db
                                                                   :editor.input
                                                                   {:cursor 3
                                                                    :text "界é"}
                                                                   {:columns 1}))
                                    (assert (and (and (and (= (length narrow.lines)
                                                              2)
                                                           (= (. narrow.lines 1
                                                                 :spans 1 :text)
                                                              ""))
                                                      (= narrow.cursor.row 2))
                                                 (= narrow.cursor.byte 0))
                                            "narrow input wrapping lost its cursor or prompt budget")
                                    (local cjk
                                           (misa.components.render db
                                                                   :editor.input
                                                                   {:cursor 3
                                                                    :text "界"}
                                                                   {:columns 2}))
                                    (assert (and (= (. cjk.lines 1 :spans 2
                                                       :text)
                                                    "界")
                                                 (= cjk.cursor.byte 3))
                                            "narrow prompt hid a wide grapheme")
                                    (local crlf
                                           (misa.layout.wrap-input "a\r\nb\rc"
                                                                   6 3 "> "
                                                                   :plain
                                                                   :accent))
                                    (assert (and (and (= (length crlf.lines) 3)
                                                      (= crlf.cursor.row 2))
                                                 (= crlf.cursor.byte 2))
                                            (.. "CRLF cursor was not translated through newline normalization: "
                                                (length crlf.lines) ","
                                                crlf.cursor.row ","
                                                crlf.cursor.byte))
                                    (local inside-crlf
                                           (misa.layout.wrap-input "a\r\nb" 6 2
                                                                   "> " :plain
                                                                   :accent))
                                    (assert (and (= inside-crlf.cursor.row 2)
                                                 (= inside-crlf.cursor.byte 2))
                                            "cursor inside CRLF was not normalized coherently")
                                    (local picker
                                           (misa.components.render db :picker
                                                                   {:columns [{:active true
                                                                               :id :all
                                                                               :lines (misa.choices.row-lines {:active true
                                                                                                               :description :option
                                                                                                               :hotkey :alt+1
                                                                                                               :label "界界界界 wrapped tail"
                                                                                                               :marker ">"
                                                                                                               :value :long}
                                                                                                              28)
                                                                               :rows [{:value :long}]
                                                                               :title :Choices
                                                                               :width 28}]
                                                                    :height 8
                                                                    :hints {}
                                                                    :input {:cursor 0
                                                                            :text ""
                                                                            :title :Pick}
                                                                    :panel_height 6
                                                                    :preview {:height 0
                                                                              :lines {}}
                                                                    :width 28
                                                                    :x 0}))
                                    (var text "")
                                    (each [_ line (ipairs picker.lines)]
                                      (each [_ part (ipairs line.spans)]
                                        (set text (.. text part.text))))
                                    (assert (text:find :tail 1 true)
                                            "picker option was truncated instead of wrapped")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text "unicode layout"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.unicode-layout declarations
    {}))
