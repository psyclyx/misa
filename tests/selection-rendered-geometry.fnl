(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(each [_ name (ipairs [:json :layout :markdown :component/markdown :selection_document])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) {}))
(local specs ((. (fennel.dofile :extensions/selection.fnl) :setup)))
(local events {})
(each [_ effect (ipairs specs.fx)]
  (when (= effect.type :register/event) (tset events effect.name effect.handler)))
(misa._setup_effects specs)
(misa._setup_effects {:fx [{:type :register/selection-source :id :rendered
                          :value (fn [db] [db.document])
                          :layout (fn [_ document terminal]
                                    (misa.markdown_view.render (misa.markdown.parse document.text)
                                                               {:columns terminal.columns}))}]})
(fn transition [db type action columns]
  (local before (misa.json.encode db))
  (local result ((. events type) db {: type : action} {:terminal {:columns (or columns 10)}}))
  (assert (= before (misa.json.encode db)) "rendered navigation mutated prior state")
  (misa.patch db (or result.patch {})))
(fn open [text]
  (transition {:document (misa.selection_document :doc :doc text)} :selection/open))
(fn act [db action columns] (transition db :selection/action action columns))
(fn selected [db]
  (local slice (misa.selection_projection db))
  (slice.text:sub (+ slice.first 1) slice.last))
(local words (act (act (act (open "one **two** three four") :child) :child) :child))
(assert (= (selected words) :one))
(local two (act words :right))
(assert (= (selected two) "**two**"))
(assert (= (selected (act two :right)) "**two**") "horizontal motion crossed a wrapped row")
(assert (= (selected (act two :down)) :four) "vertical motion ignored actual wrapped column")
(assert (= (selected (act (act two :down) :up)) "**two**"))
(assert (= (selected (act two :right 40)) :three) "resize kept stale source geometry")
(assert (= (selected (act two :down 40)) "**two**") "single visible row moved vertically")
;; Rich span boundaries and link destinations never distort source coordinates.
(local rich-source "al**ph**a [beta](beta) gamma")
(local rich (act (act (act (open rich-source) :child) :child) :child))
(assert (= (selected rich) "al**ph**a"))
(assert (= (selected (act rich :right 10)) "[beta](beta)"))
(assert (= (selected (act (act rich :right 10) :down 10)) :gamma))
;; List indentation and bullets use the same prefix spans that are painted.
(local list (act (act (act (act (open "- first second third\n- next item") :child) :child) :child) :child))
(assert (= (selected list) "-"))
(local first (act list :right 14))
(assert (= (selected first) :first))
(assert (= (selected (act first :right 14)) :second))
(assert (= (selected (act (act first :right 14) :down 14)) :third))
;; Every literal source span maps back exactly, even with escapes and repeats.
(each [_ source (ipairs ["one **two** three four" rich-source "\\* one [one](one) one"
                         "- [x] x **x**" "| a | **b** |\n|---|---|\n| c | d |"
                         "```zig\nconst x = 1;\n```" "first\n  second third"]) ]
  (local document (misa.markdown.parse source))
  (each [_ line (ipairs (misa.markdown_view.render document {:columns 12}))]
    (each [_ span (ipairs line.spans)]
      (when (and span.source span.source_start)
        (local original (source:sub (+ span.source_start 1) span.source_end))
        (assert (or (= original span.text) (and (= original "\n") (= span.text " ")))
                (.. "bad source span: " (fennel.view span) " in " source))))))
(output "rendered selection geometry passed\n")
;; A tool's arguments and result use the same transcript section, but a frozen
;; selection keeps its original source owner when the result arrives.
(misa._setup (fennel.dofile :extensions/values.fnl) {})
(local message-specs ((. (fennel.dofile :extensions/messages.fnl) :setup) {:config {}}))
(var tool-documents nil)
(each [_ effect (ipairs message-specs.fx)]
  (when (= effect.type :register/selection-source) (set tool-documents effect.value)))
(local pending {:messages {:blocks [{:id :call :response_id :response :kind :tool_call
                                   :text "" :arguments {:path :settings}}]}})
(local argument-document (. (tool-documents pending) 1))
(assert (= argument-document.source_part :args))
(local selected-args (transition {:document argument-document :messages pending.messages} :selection/open))
(local completed (misa.patch selected-args {:messages {:blocks (misa.replace [{:id :call :response_id :response
                                                                             :kind :tool_call :text "" :result "new output"}])}}))
(assert (= (. (misa.selection_projection completed) :source_part) :args))
(assert (= (. (misa.selection_projection completed) :text) argument-document.text))
(local result-document (. (tool-documents completed) 1))
(assert (= result-document.id argument-document.id))
(assert (= result-document.source_part :result))
(assert (= result-document.text "new output"))

(local checkbox (misa.markdown_view.render (misa.markdown.parse "- [x] item") {:columns 20}))
(local marker (accumulate [found nil _ span (ipairs (. checkbox 1 :spans))]
                (or found (when span.selection_marker span))))
(assert (= marker.source_start 0))
(assert (= marker.source_end 6) "flow replaced source marker length with glyph bytes")
(set misa.theme_style (fn [] {:background :selected}))
(local rule-db (open "---"))
(local rule (misa.selection_decorate rule-db :doc "---"
                                    (misa.markdown_view.render (misa.markdown.parse "---") {:columns 20})))
(each [_ span (ipairs (. rule 1 :spans))]
  (assert (= span.style.background :selected) "nonliteral source lost partial selection paint"))
