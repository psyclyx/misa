(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local declarations (require :misa.definitions))
(each [_ name (ipairs [:json :themes :theme/default :components :actions :layout
                       :markdown :component/markdown :values :component/content :component/truncation :tool/presentations :component/tool
                       :selection/document :selection])]
  (app.include (fennel.dofile (.. :extensions/ name :.fnl)) {:config {}}))
(app.install)
(fn line-text [line]
  (table.concat (icollect [_ item (ipairs line.spans)] item.text)))
(fn render [source columns]
  (misa.markdown.view.render (misa.markdown.parse source) {:columns (or columns 80)}))
(local code (render "```zig\none\ntwo\n```"))
(assert (= (length code) 2) "code retained frame rows")
(assert (: (line-text (. code 1)) :find "1  one" 1 true))
(assert (: (line-text (. code 2)) :find "2  two" 1 true))
(local diff (render "```diff\n@@ -10,2 +20,2 @@\n-old\n+new\n same\n```"))
(assert (: (line-text (. diff 2)) :find "10  -old" 1 true))
(assert (: (line-text (. diff 3)) :find "20  +new" 1 true))
(assert (: (line-text (. diff 4)) :find "21   same" 1 true))
(assert (not (: (line-text (. diff 4)) :find "11   21" 1 true)))
(local deleted (accumulate [found nil _ part (ipairs (. diff 2 :spans))]
                (or found (when (= part.text "-old") part))))
(assert (accumulate [found false _ style (ipairs deleted.style)] (or found (= style :diff.removed))))
(each [_ width (ipairs [1 2 4 10 20])]
  (each [_ line (ipairs (render "```diff\n-old\n+new\n```" width))]
    (assert (<= (misa.layout.width (line-text line)) width))))
(local original-projection misa.selection.state)
(local original-theme misa.themes.style)
(set misa.themes.style (fn [] {:background :selected}))
(set misa.selection.state (fn [] {:id :doc :first 0 :last 5 :text "- one"}))
(local decorated (misa.selection.decorate {} :doc "- one" (render "- one")))
(local bullet (accumulate [found nil _ part (ipairs (. decorated 1 :spans))] (or found (when (= part.text "• ") part))))
(assert (and bullet bullet.style.background) "list bullet was not selected")
(set misa.selection.state original-projection)
(set misa.themes.style original-theme)
;; A successful summary exposes useful content with a bounded visual height.
(fn tool [model context]
  (misa.components.render {:components {:roles {}} :themes {:active :default}}
                        :transcript.tool_call model context))
(local result (tool {:kind :tool_call :name :read_file :collapsed true
                     :arguments {:path :file}
                     :result "one\ntwo\nthree\nfour\nfive"} {:columns 80}))
(local text (table.concat (icollect [_ line (ipairs result.lines)] (line-text line)) "\n"))
(assert (text:find :one 1 true))
(assert (not (text:find :four 1 true)))
(assert (text:find "…" 1 true))
(output "transcript rendering details passed\n")
(local summarized (tool {:kind :tool_call :name :read_file :collapsed true
                         :arguments {} :result "raw verbose result"
                         :summary "Read 24 lines from settings."} {:columns 80}))
(local summarized-text (table.concat (icollect [_ line (ipairs summarized.lines)] (line-text line)) "\n"))
(assert (summarized-text:find "Read 24 lines from settings." 1 true))
(assert (not (summarized-text:find "raw verbose result" 1 true)))

;; component.tool's optional JSON extension is not a rendering dependency.
(local json misa.json)
(set misa.json nil)
(local no-json (tool {:kind :tool_call :name :read_file :collapsed true
                     :arguments {:path :settings} :result :ok}
                    {:columns 80}))
(assert (: (table.concat (icollect [_ line (ipairs no-json.lines)] (line-text line)) "\n")
           :find "read_file  settings" 1 true))
(set misa.json json)
