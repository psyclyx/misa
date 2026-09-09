(local fennel (require :fennel))
(local output print)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(each [_ name (ipairs [:misa.json :misa.ui.themes :misa.ui.themes.default :misa.ui.components :misa.ui.layout :misa.markdown
                       :misa.markdown.render :misa.ui.values :misa.ui.components.content :misa.ui.components.truncation
                       :misa.transcript.tools :misa.transcript.tools.render])]
  (app.define ((. (require name) :build) {:config {}})))
(app.define (definitions.build :fixture [{:catalog :components :id :fixture.text :value {:render (fn [] {:lines [{:spans [{:text :replacement}]}]})}}]))
(app.define (definitions.build :fixture [(let [definition (fn [model]
                                    {:arguments [{:role :content.text :model {:text model.arguments.query}}]
                                     :result {:role :content.text :model {:text model.result}}})] {:catalog :tool-presentations :id :external :value definition})]))
(app.install)
(local db {:components {:roles {}} :themes {:active :default}})
(fn text [rendered]
  (table.concat (icollect [_ line (ipairs rendered.lines)]
                  (table.concat (icollect [_ part (ipairs line.spans)] part.text))) "\n"))
(fn tool [model columns]
  (misa.components.render db :transcript.tool_call model {:columns (or columns 80) :interactive true}))
(local model {:kind :tool_call :name :shell :description "An excessive description"
              :arguments {:command "echo hello"} :collapsed true
              :result "one\ntwo\nthree\nfour\nfive" :status :success :elapsed_ms 1250 :started_wall_ms 10000})
(local view (tool model))
;; Newline termination is shared code-view behavior; commands do not opt in
;; to missing-newline annotations. Padding belongs to the containing tool.
(fn shell-output [value]
  (tool {:kind :tool_call :name :shell :arguments {:command "echo hello"}
         :result value :status :success :collapsed false}))
(fn row-text [line]
  (table.concat (icollect [_ part (ipairs line.spans)] part.text)))
(local terminated (shell-output "hello\n"))
(assert (= (length terminated.lines) 6))
(each [_ index (ipairs [2 4 6])]
  (assert (= (: (row-text (. terminated.lines index)) :gsub "%s+$" "") "┃"))
  (assert (not (. terminated.lines index :source_part))))
(assert (not (: (text terminated) :find "No newline" 1 true)))
(local unterminated (shell-output "hello"))
(assert (: (text unterminated) :find "\\ No newline at end of output" 1 true))
(var annotations 0)
(each [_ line (ipairs unterminated.lines)]
  (when line.annotation
    (set annotations (+ annotations 1))
    (assert (= line.source_part :result))
    (each [_ part (ipairs line.spans)] (assert (not part.source)))))
(assert (= annotations 1))
(assert (= (length (. (shell-output "hello\n\n") :lines)) (+ (length terminated.lines) 1)))
(local summary-view (tool (misa.patch model {:summary "Five output lines"})))
(assert (= (text summary-view) (text view)) "shell preview must retain the actual output tail")
(local command-view (misa.components.render db :content.code
                      {:text "echo hello\n" :numbered false} {:columns 80}))
(assert (= (length command-view.lines) 1))
(assert (not (: (text command-view) :find "No newline" 1 true)))
(local rendered (text view))
(assert (rendered:find "echo hello" 1 true))
(assert (rendered:find "2 lines hidden" 1 true))
(assert (rendered:find "No newline" 1 true))
(assert (not (rendered:find "one" 1 true)))
(assert (not (rendered:find "two" 1 true)))
(assert (< (rendered:find "2 lines hidden" 1 true) (rendered:find "three" 1 true)))
(assert (< (rendered:find "three" 1 true) (rendered:find "five" 1 true)))
(local expanded (text (tool (misa.patch model {:collapsed false}))))
(assert (expanded:find "one" 1 true))
(assert (not (expanded:find "lines hidden" 1 true)))
(local failed-shell (text (tool (misa.patch model {:is_error true :status :error}))))
(assert (failed-shell:find "five" 1 true))
(assert (not (failed-shell:find "one" 1 true)))
;; Both ends use the same truncation component; annotations follow their
;; preceding content and do not count as omitted output rows.
(each [_ tail (ipairs [false true])]
  (local clipped (misa.components.render db :content.truncated
                  {:lines [{:spans [{:text "first"}]} {:spans [{:text "last"}]}
                           {:annotation true :spans [{:text "annotation" :source false}]}]
                   :limit 1 : tail} {:columns 80}))
  (assert (: (text clipped) :find "1 line hidden" 1 true))
  (assert (= (not= (: (text clipped) :find "annotation" 1 true) nil) tail)))
(assert (not (rendered:find "excessive" 1 true)))
(assert (not (rendered:find "00:00:10" 1 true)))
(assert (. view.lines 1 :spans 1 :style :background) "railed title lost its surface")
(assert (: (text {:lines [(. view.lines 1)]}) :find "◇ shell" 1 true))
(assert (not (: (text {:lines [(. view.lines 1)]}) :find "echo hello" 1 true)))
(assert (not (rendered:find "✓ success" 1 true)))
;; File rows and ordinary code share geometry and surfaces, rather than
;; independently approximating the same appearance.
(local code-context {:columns 60 :interactive true})
(local code-view (misa.components.render db :content.code {:text "hello\nworld" :style :tool} code-context))
(local rows-view (misa.components.render db :content.lines
                   {:rows [{:number 1 :text :hello :source_start 10}
                           {:number 2 :text :world :source_start 30}] :style :tool} code-context))
(assert (= (text code-view) (text rows-view)) "file preview diverged from code-block layout")
(each [i line (ipairs code-view.lines)]
  (each [j part (ipairs line.spans)]
    (assert (= (misa.json.encode part.style) (misa.json.encode (. rows-view.lines i :spans j :style)))
            "file preview diverged from code-block styling")))
(local shell-binding (misa.tools.presentation model))
(assert (= shell-binding.result.role :content.code))
(assert (= shell-binding.result.model.numbered false))
(local code-background (. (misa.themes.style db :surface.code) :background))
(var shell-output-code false)
(each [_ line (ipairs view.lines)]
  (each [_ part (ipairs line.spans)]
    (when (and (= line.source_part :result) (= part.text :three))
      (assert (= (misa.json.encode part.style.background) (misa.json.encode code-background))
              (.. "shell output lost its code surface: " (fennel.view {:actual part.style.background :expected code-background})))
      (set shell-output-code true))))
(assert shell-output-code)
(local pending-edit (misa.tools.presentation {:kind :tool_call :name :edit_file
                      :arguments {:path :file :old_text :legacy :new_text :replacement
                                  :snapshot :HASH :start :21#HASH}}))
(assert (= (length pending-edit.arguments) 1))
(assert (= (. pending-edit.arguments 1 :role) :content.code))
(assert (= (. pending-edit.arguments 1 :model :text) :replacement))
(local highlighted-model (collect [key value (pairs model)] key value))
(set highlighted-model.syntax {:captures {0 [{:start_byte 0 :end_byte 4 :capture :function}]}})
(local highlighted (tool highlighted-model))
(var highlighted-command false)
(each [_ line (ipairs highlighted.lines)]
  (each [_ part (ipairs line.spans)]
    (when (= part.text :echo)
      (assert (= line.source_part :args))
      (set highlighted-command true))))
(assert highlighted-command "shell code block lost its syntax captures")
(local failure {:kind :tool_call :name :read_file :arguments {:path :folder}
                :is_error true :status :error
                :result "This path is a directory. Use list_directory.\nDetails: IsDir"})
(assert (not (: (text (tool (misa.patch failure {:collapsed true}))) :find "Details:" 1 true)))
(assert (: (text (tool (misa.patch failure {:collapsed false}))) :find "Details: IsDir" 1 true))
(each [_ line (ipairs view.lines)]
  (when line.omitted_lines
    (assert (= line.omitted_lines 2))
    (each [_ part (ipairs line.spans)]
      (assert (not part.source))
      (assert part.style.background "railed truncation lost its surface"))))
(local unknown (tool {:kind :tool_call :name :unknown :arguments {:path :settings :count 3}
                     :collapsed true :result :ok}))
(assert (: (text unknown) :find "path  settings" 1 true))
(assert (: (text unknown) :find "count  3" 1 true))
;; Selection renders the frozen canonical document, bypassing transformed views.
(local selected (tool (misa.patch model {:collapsed false :selection_source :result :selection_text "four\nfive"})))
(var found false)
(each [_ line (ipairs selected.lines)]
  (when (= line.source_part :result)
    (each [_ part (ipairs line.spans)]
      (when (and part.source (= part.text :five))
        (assert (= part.source_start 5))
        (assert (= part.source_end 9))
        (set found true)))))
(assert found "selected result lost canonical offsets")
(each [_ width (ipairs [1 2 4 10 20 80])]
  (each [_ line (ipairs (. (tool model width) :lines))]
    (assert (<= (misa.layout.width (table.concat (icollect [_ part (ipairs line.spans)] part.text))) width))))
;; Generic children honor role swaps and invalidate the parent's cached source.

(local item {:id :tool :role :transcript.tool_call
             :model {:kind :tool_call :name :unknown :collapsed true :result :original}})
(local before (misa.components.project db :nested [item] {:columns 80}))
(local changed (misa.components.swap db :content.text :fixture.text))
(local after (misa.components.project changed :nested [item] {:columns 80}))
(assert (: (text (. before.views 1)) :find :original 1 true))
(assert (: (text (. after.views 1)) :find :replacement 1 true))
;; File wire format is decoded by its binding, never by the generic row view.
(local raw "snapshot ABCD\n21#CAFE|hello\n22#BEEF|world\n23#ABCD|third\n24#DCBA|fourth\n")
(local file-model {:kind :tool_call :name :read_file :arguments {:path :file}
                   :collapsed true :result raw})
(local file-view (tool file-model))
(assert (not (: (text file-view) :find :snapshot 1 true)))
(assert (not (: (text file-view) :find :CAFE 1 true)))
(assert (: (text file-view) :find "21  hello" 1 true))
(assert (: (text file-view) :find "1 line hidden" 1 true))
(var file-source false)
(each [_ line (ipairs file-view.lines)]
  (each [_ part (ipairs line.spans)]
    (when (= part.text :hello)
      (assert (= (raw:sub (+ part.source_start 1) part.source_end) :hello))
      (set file-source true))))
(assert file-source)
(local edited-text "@@ -2,2 +2,3 @@\n one\n two\n+Added line\n\nsnapshot ABCD\n1#CAFE|Title\n2#BEEF|one\n3#ABCD|two\n4#DCBA|Added line\n")
(local edit-model {:kind :tool_call :name :edit_file :status :success
                   :arguments {:path :file :position :after :start :3#ABCD :new_text "Added line"}
                   :result edited-text :collapsed true :summary "summary must not hide the diff"})
(local edit-view (tool edit-model))
(local edit-text (text edit-view))
(assert (edit-text:find "4  +Added line" 1 true) "insertion did not show its actual new-file line")
(assert (edit-text:find "2   one" 1 true) "diff did not retain its single file-line gutter")
(assert (not (edit-text:find "2 2" 1 true)) "diff displayed two line numbers")
(assert (not (edit-text:find :snapshot 1 true)))
(assert (not (edit-text:find :ABCD 1 true)))
(assert (not (edit-text:find "1 │ Added line" 1 true)))
(assert (not (edit-text:find "summary must" 1 true)))
(var edit-code false)
(each [_ line (ipairs edit-view.lines)]
  (each [_ part (ipairs line.spans)]
    (when (= part.text "+Added line")
      (assert (= (misa.json.encode part.style.background) (misa.json.encode code-background)) "edit diff diverged from the shared code surface")
      (set edit-code true))))
(assert edit-code)
(each [_ width (ipairs [1 2 4 10 20 80])]
  (each [_ line (ipairs (. (tool edit-model width) :lines))]
    (assert (<= (misa.layout.width (table.concat (icollect [_ part (ipairs line.spans)] part.text))) width))))
(local old-edit (tool {:kind :tool_call :name :edit_file :result raw :collapsed true}))
(assert (: (text old-edit) :find "21  hello" 1 true) "old edit snapshots differ from read_file")
(local file-selected (tool (misa.patch file-model {:collapsed false :selection_source :result :selection_text raw})))
(assert (: (text file-selected) :find "21#CAFE|hello" 1 true))
(assert (: (text file-selected) :find "snapshot ABCD" 1 true))
;; External tools can bind ordinary components without changing the wrapper.

(assert (: (text (tool {:kind :tool_call :name :external :arguments {:query :query-value}
                       :collapsed true :result :answer}))
           :find :query-value 1 true))
(assert (not (rendered:find "┌" 1 true)))
(assert (not (rendered:find "1 │ echo" 1 true)))
;; Both standalone results and argument selections render frozen source text.
(local standalone (misa.components.render db :transcript.tool_result
                    {:kind :tool_result :text :old :result :latest :collapsed false
                     :selection_source :result :selection_text "frozen\nresult"}
                    {:columns 80 :interactive true}))
(assert (: (text standalone) :find "frozen" 1 true))
(assert (not (: (text standalone) :find :latest 1 true)))
(local selected-args (tool (misa.patch model {:collapsed false :selection_source :args
                                             :selection_text "{command=frozen-command}"})))
(assert (: (text selected-args) :find "{command=frozen-command}" 1 true))
(each [_ line (ipairs selected-args.lines)]
  (when (= line.source_part :result)
    (each [_ part (ipairs line.spans)] (assert (not part.source)))))
;; Completed status is conveyed by the surface; live states keep their labels.
(each [status marker (pairs {:success "" :error "" :cancelled "⊘ cancelled"
                             :running "… running" :pending "… pending"})]
  (local status-view (tool {:kind :tool_call :name :read_file :arguments {:path :settings}
                           : status :is_error (= status :error) :collapsed true :result :ok}))
  (local heading (text {:lines [(. status-view.lines 1)]}))
  (assert (heading:find "◇ read_file  settings" 1 true))
  (assert (heading:find marker 1 true))
  (assert (= (length status-view.lines) 4) "tool should contain header, padded result, and no argument body")
  (each [_ line (ipairs status-view.lines)]
    (each [_ part (ipairs line.spans)]
      (assert part.style.background "railed tool row lost its surface"))))
(local multiline (tool {:kind :tool_call :name :shell :arguments {:command "echo first\necho second"}
                        :status :running :collapsed false}))
(assert (: (text multiline) :find "echo first" 1 true))
(assert (: (text multiline) :find "echo second" 1 true))
(assert (not (: (text {:lines [(. multiline.lines 1)]}) :find "echo first" 1 true)))
(local writing (tool {:kind :tool_call :name :write_file :arguments {:path :settings :content "new contents"}
                      :status :success :collapsed false :result :written}))
(assert (: (text {:lines [(. writing.lines 1)]}) :find "◇ write_file  settings" 1 true))
(assert (: (text writing) :find "new contents" 1 true))
(output "tool presentation passed")
