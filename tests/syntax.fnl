;; Pure projection consumes async captures; stale streaming completions cannot
;; recolor newer text or create an unbounded queue of highlighting requests.
(local fennel (require :fennel))
(local output io.write)
(local context
       {:argv []
        :config {:themes {:persist false} :components {:persist false}}})

(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(each [_ name (ipairs [:keybindings
                       :themes
                       :theme/default
                       :components
                       :layout
                       :markdown
                       :tool_presentations
                       :syntax
                       :component/markdown
                       :values :component/truncation :component/group :component/message :component/content :component/tool
                       :messages])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))

(var db nil)
(var expected-syntax nil)
(var syntax-input-checked false)
(misa._setup_effects {:fx [{:type :register/event
                            :name :test/read
                            :handler (fn [value] (set db value))}]})

(misa._setup_effects {:fx [{:type :register/event
                            :name :syntax/completed
                            :handler (fn [_ event]
                                       (when event.reject
                                         (error "reject completion"))
                                       (when event.mutate
                                         (set (. event.data 1 :capture)
                                              :comment)))}
                           {:type :register/event
                            :name :transcript/reset
                            :handler (fn [_ event]
                                       (when event.reject
                                         (error "reject reset")))}]})

(misa._setup_effects {:fx [{:type :register/component
                            :id :test.syntax-input
                            :value {:render (fn [model]
                                              (assert (= (type model.syntax)
                                                         :table))
                                              (local resolved model.syntax)
                                              (assert (= resolved expected-syntax)
                                                      "component boundary copied syntax projection")
                                              (assert (= resolved.document
                                                         expected-syntax.document)
                                                      "component boundary copied the derived document")
                                              (each [key data (pairs expected-syntax.captures)]
                                                (assert (= (. resolved.captures
                                                              key)
                                                           data)
                                                        "component boundary copied derived captures"))
                                              (set syntax-input-checked true)
                                              {:lines []})}}]})

(misa._seal context)
(local terminal {:interactive true :columns 80 :lines 24})
(local requests [])
(var commits 0)
(fn dispatch [event]
  (local queue [event])
  (var at 1)
  (while (. queue at)
    (local fx (misa._dispatch (. queue at) terminal
                              {:wall_ms 0 :monotonic_ms 0}))
    (misa._commit)
    (each [_ effect (ipairs fx)]
      (match effect.type
        :dispatch (table.insert queue effect.event)
        :syntax/highlight (table.insert requests effect)
        :view/commit (set commits (+ commits 1))
        _ (error (.. "unexpected effect: " effect.type))))
    (set at (+ at 1)))
  (misa._dispatch {:type :test/read} terminal {:wall_ms 0 :monotonic_ms 0})
  (misa._commit)
  (misa._project terminal {:wall_ms 0 :monotonic_ms 0})
  (misa._commit_projection))

(fn start []
  (dispatch {:type :transcript/response-start :response_id :reply})
  (dispatch {:type :transcript/block-start
             :response_id :reply
             :block_id :body
             :kind :assistant}))

(fn delta [text]
  (dispatch {:type :transcript/block-delta
             :response_id :reply
             :block_id :body
             : text}))

(local captures [{:start_byte 0 :end_byte 5 :capture :keyword}])
(fn complete [index ok]
  (dispatch {:type :syntax/completed
             :id (. requests index :id)
             : ok
             :mutate true
             :data (misa.snapshot captures)}))

(var view nil)
(fn render []
  (local model (. db.messages.blocks 1))
  (local syntax (assert (misa.syntax_projection (misa.syntax_projections db) model)))
  (set view (misa.markdown_view.project (or model.text (table.concat model.chunks))
                                       (misa.patch syntax {:columns 80}) view))
  view.lines)

(fn keyword? [lines]
  (accumulate [found false _ line (ipairs lines) &until found]
    (accumulate [found false _ span (ipairs line.spans) &until found]
      (accumulate [found false _ token (ipairs (if (= (type span.style) :string)
                                                   [span.style]
                                                   (or span.style [])))
                   &until found]
        (= token :syntax.keyword)))))

;; Equal revision numbers in independent snapshots are not equal dependencies.
(let [source "```lua\nlocal value = 1\n```"
      document (misa.markdown.parse source)
      other-document (misa.markdown.parse source)
      options {:document document :columns 80 :revision 1}
      plain-view (misa.markdown_view.project source options)
      plain plain-view.lines
      colored-options {:document document :columns 80 :revision 1
                       :captures {(. document.blocks 1 :source_start)
                                  [{:start_byte 0 :end_byte 5 :capture :keyword}]}}
      colored-view (misa.markdown_view.project source colored-options plain-view)
      colored colored-view.lines]
  (assert (not (keyword? plain)))
  (assert (keyword? colored) "equal revision hid changed capture input")
  (assert (not= (. plain-view.entries 1) (. colored-view.entries 1))
          "capture branch reused a mutable layout entry")
  (assert (= (. plain-view.entries 1 :captures) nil)
          "capture branch modified prior layout dependencies")
  (assert (= colored-view (misa.markdown_view.project source colored-options colored-view))
          "identical explicit dependencies discarded layout")
  (assert (= colored-view (misa.markdown_view.project source {:document document :columns 80 :revision 2
                                          :captures colored-options.captures} colored-view))
          "revision bookkeeping invalidated unchanged layout inputs")
  (local restored (misa.markdown_view.project source options colored-view))
  (assert (not (keyword? restored.lines))
          "retained snapshot reused another snapshot's captures")
  (local replaced (misa.markdown_view.project source {:document other-document :columns 80 :revision 1} restored))
  (assert (not= replaced restored) "replacement document identity was ignored")
  (assert (= plain-view (misa.markdown_view.project source options plain-view))
          "branching layout changed the original projection")
  (assert (and (not (keyword? plain-view.lines)) (keyword? colored-view.lines))
          "branching layout mutated a retained projection"))

(dispatch {:type :app/start})
(start)
(delta "intro\n\n```lua\nlocal a")
(assert (= (length requests) 1))
(assert (not (keyword? (render))))
(delta " = 1")
(assert (= (length requests) 1)
        "streaming queued work behind an in-flight request")

(complete 1 true)
(assert (= (length requests) 2)
        "stale completion did not schedule latest source")

(assert (not (keyword? (render))) "stale captures colored changed text")
(local plain (render))
(local rejected (pcall misa._dispatch
                       {:type :syntax/completed
                        :id (. requests 2 :id)
                        :ok true
                        :data captures
                        :reject true} terminal
                       {:wall_ms 0 :monotonic_ms 0}))

(assert (not rejected))
(dispatch {:type :test/read})
(assert (not (keyword? (render)))
        "aborted completion exposed memoized captures")

(complete 2 true)
(local colored (render))
(assert (= (. plain 1) (. colored 1))
        "capture completion relaid out an unchanged paragraph")

(assert (not= plain colored)
        "capture completion did not invalidate render cache")

(assert (keyword? colored)
        "capture input did not produce semantic syntax styles")

(assert (= colored (render)) "unchanged captures discarded cached rendering")
(each [_ entry (pairs db.syntax.documents)]
  (assert (= entry.document.kind :document))
  (each [_ slot (ipairs entry.slots)]
    (assert (= (. slot.data 1 :capture) :keyword)
            "accepted capture payload is not owned by syntax state")))

(local reset-ok (pcall misa._dispatch {:type :transcript/reset :reject true}
                       terminal {:wall_ms 0 :monotonic_ms 0}))

(assert (not reset-ok))
(dispatch {:type :test/read})
(assert (keyword? (render)) "aborted reset destroyed accepted captures")
(dispatch {:type :ui/redraw})
(assert (= (length requests) 2) "projection or redraw scheduled syntax work")
(assert (= colored (render)) "unrelated transaction discarded cached rendering")
(set expected-syntax (misa.syntax_projection (misa.syntax_projections db) (. db.messages.blocks 1)))
(set (. db.components.roles :transcript.assistant) :test.syntax-input)
(misa.transcript_projection db terminal)
(assert syntax-input-checked "syntax component failed before validating its shared inputs")
(set (. db.components.roles :transcript.assistant) nil)
;; Real component models preserve the explicit input and require no
;; synchronous native capability during projection.
(assert (> (length (misa.transcript_projection db terminal)) 0))
(local frozen-text (table.concat (. db.messages.blocks 1 :chunks)))
(delta "\nlocal b")
(assert (= (length requests) 3))
(assert (not (keyword? (render))) "new source retained stale captures")
(local previous-selection misa.selection_projection)
(local live-block (. db.messages.blocks 1))
(set misa.selection_projection (fn [_ id]
                                 (when (= id "5:replybody")
                                   {: id :text frozen-text :first 0 :last (length frozen-text)})))
(local frozen-lines (misa.transcript_projection db terminal))
(local frozen-output (table.concat (icollect [_ line (ipairs frozen-lines)]
                                    (table.concat (icollect [_ span (ipairs line.spans)] span.text))) "\n"))
(assert (frozen-output:find "local a" 1 true))
(assert (not (frozen-output:find "local b" 1 true))
        "frozen selection rendered the live syntax document")
(assert (= live-block (. db.messages.blocks 1)))
(assert (= (table.concat live-block.chunks) (.. frozen-text "\nlocal b"))
        "selection changed canonical stream chunks")
(set misa.selection_projection previous-selection)
(local live-lines (misa.transcript_projection db terminal))
(local live-output (table.concat (icollect [_ line (ipairs live-lines)]
                                  (table.concat (icollect [_ span (ipairs line.spans)] span.text))) "\n"))
(assert (live-output:find "local b" 1 true) "leaving selection did not restore live syntax")
(dispatch {:type :transcript/reset})
(dispatch {:type :transcript/updated :response_id :reply :block_id :body})
(assert (= (next db.syntax.documents) nil) "queued notification resurrected a reset document")
(complete 3 true)
(assert (= (length requests) 3) "reset resurrected an old syntax request")
(start)
(delta "```lua\nlocal c")
(assert (= (length requests) 4))
(dispatch {:type :transcript/updated :response_id :reply :block_id :body})
(assert (= (length requests) 4) "duplicate notification bypassed in-flight coalescing")
(assert (not= (. requests 3 :id) (. requests 4 :id))
        "reset reused an in-flight ID")

(complete 4 false)
(dispatch {:type :ui/redraw})
(assert (= (length requests) 4)
        "failed highlighting retried without a source change")

(assert (not (keyword? (render))))
(set terminal.interactive false)
(dispatch {:type :transcript/user :text "```lua\nlocal d\n```"})
(local written commits)
(assert (= (length requests) 4)
        "headless transcript scheduled unused highlighting")

(dispatch {:type :syntax/completed :id :ignored :ok true :data captures})
(assert (= commits written)
        "highlight completion duplicated noninteractive output")

;; Collection consumers resolve the subscription once, not once per block.
(local snapshot-service misa.syntax_projections)
(var snapshots 0)
(set misa.syntax_projections (fn [state]
                              (set snapshots (+ snapshots 1))
                              (snapshot-service state)))
(local many (misa.patch db {:messages {:blocks (misa.replace
                                               (fcollect [index 1 300]
                                                 {:id (tostring index) :kind :assistant :text "plain"}))}}))
(misa.transcript_projection many terminal)
(assert (= snapshots 1) "transcript repeated syntax subscription lookups per block")
(local snapshot (snapshot-service db))
(local saved-sub misa.sub)
(set misa.sub (fn [] (error "pure syntax lookup entered subscription engine")))
(misa.syntax_projection snapshot (. db.messages.blocks 1))
(set misa.sub saved-sub)
(set misa.syntax_projections snapshot-service)
(set terminal.interactive true)
(local before-shell (length requests))
(dispatch {:type :transcript/response-start :response_id :shell-reply})
(dispatch {:type :transcript/block-start :response_id :shell-reply :block_id :shell
           :kind :tool_call :name :shell :call_id :shell-call})
(dispatch {:type :transcript/block-delta :response_id :shell-reply :block_id :shell
           :arguments {:command "echo hello"}})
(assert (= (length requests) (+ before-shell 1)) "shell command did not request syntax highlighting")
(assert (= (. requests (length requests) :language) :sh))
(assert (= (. requests (length requests) :source) "echo hello"))
(output "async syntax regressions passed\n")
