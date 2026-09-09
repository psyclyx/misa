(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(each [_ name (ipairs [:misa.json :misa.ui.layout :misa.markdown :misa.markdown.render :misa.selection.document])]
  (app.define ((require name) {})))
(local specs ((fennel.dofile :extensions/misa/selection/init.fnl) {}))
(local events {})
(each [_ effect (pairs specs.events)] (tset events effect.event effect.handler))

(var layout-calls 0)

(app.define specs)
(app.define (definitions :fixture [{:catalog :selection-sources :id :rendered :value {:documents (fn [db] [db.document]) :layout (fn [_ document terminal]
                                    (set layout-calls (+ layout-calls 1))
                                    (misa.markdown.view.render (misa.markdown.parse document.text)
                                                               {:columns terminal.columns}))}}]))
(app.define ((fennel.dofile :extensions/misa/ui/values.fnl) {}))
(app.install)
(fn transition [db type action columns]
  (local before (misa.json.encode db))
  (local terminal {:columns (or columns 10)})
  ;; Simulate geometry from the last accepted frame before delivering input.
  (local geometry (misa.selection.geometry db terminal))
  (local calls-before layout-calls)
  (local result ((. events type) db {: type : action}
                 {: terminal :presentation {:selection/geometry geometry}}))
  (assert (= layout-calls calls-before) "input invoked a presentation provider")
  (assert (= before (misa.json.encode db)) "rendered navigation mutated prior state")
  (misa.patch db (or result.patch {})))
(fn open [text]
  (transition {:document (misa.selection.document :doc :doc text)} :selection/open))
(fn act [db action columns] (transition db :selection/action action columns))
(fn selected [db]
  (local slice (misa.selection.state db))
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
(local accepted (misa.selection.geometry two {:columns 10}))
(local resized ((. events :selection/action) two {:action :right}
               {:terminal {:columns 40} :presentation {:selection/geometry accepted}}))
(assert (= (selected (misa.patch two resized.patch)) "**two**")
        "unaccepted resize changed navigation geometry")
(local streaming (misa.patch two {:messages {:above :changed :onscreen :changed :below :changed}}))
(local streamed ((. events :selection/action) streaming {:action :down}
                {:terminal {:columns 10} :presentation {:selection/geometry accepted}}))
(assert (= (selected (misa.patch streaming streamed.patch)) :four)
        "stream updates displaced frozen source geometry")
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
  (each [_ line (ipairs (misa.markdown.view.render document {:columns 12}))]
    (each [_ span (ipairs line.spans)]
      (when (and span.source span.source_start)
        (local original (source:sub (+ span.source_start 1) span.source_end))
        (assert (or (= original span.text) (and (= original "\n") (= span.text " ")))
                (.. "bad source span: " (fennel.view span) " in " source))))))
(output "rendered selection geometry passed\n")
;; A tool's arguments and result use the same transcript section, but a frozen
;; selection keeps its original source owner when the result arrives.

(local message-specs ((fennel.dofile :extensions/misa/transcript/init.fnl) {:config {}}))
(local tool-documents (. message-specs :selection-sources :transcript :documents))
(local pending {:messages {:blocks [{:id :call :response_id :response :kind :tool_call
                                   :text "" :arguments {:path :settings}}]}})
(local argument-document (. (tool-documents pending) 1))
(assert (= argument-document.source_part :args))
(local selected-args (transition {:document argument-document :messages pending.messages} :selection/open))
(local completed (misa.patch selected-args {:messages {:blocks (misa.replace [{:id :call :response_id :response
                                                                             :kind :tool_call :text "" :result "new output"}])}}))
(assert (= (. (misa.selection.state completed) :source_part) :args))
(assert (= (. (misa.selection.state completed) :text) argument-document.text))
(local result-document (. (tool-documents completed) 1))
(assert (= result-document.id argument-document.id))
(assert (= result-document.source_part :result))
(assert (= result-document.text "new output"))

(local checkbox (misa.markdown.view.render (misa.markdown.parse "- [x] item") {:columns 20}))
(local marker (accumulate [found nil _ span (ipairs (. checkbox 1 :spans))]
                (or found (when span.selection_marker span))))
(assert (= marker.source_start 0))
(assert (= marker.source_end 6) "flow replaced source marker length with glyph bytes")
(set misa.themes (or misa.themes {}))
(set misa.themes.style (fn [] {:background :selected}))
(local rule-db (open "---"))
(local rule (misa.selection.decorate rule-db :doc "---"
                                    (misa.markdown.view.render (misa.markdown.parse "---") {:columns 20})))
(each [_ span (ipairs (. rule 1 :spans))]
  (assert (= span.style.background :selected) "nonliteral source lost partial selection paint"))
