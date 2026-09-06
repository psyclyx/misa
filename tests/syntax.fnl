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
                       :syntax
                       :component/markdown
                       :component/message
                       :messages])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))

(var db nil)
(var expected-syntax nil)
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
                                                         :function))
                                              (local resolved (model.syntax))
                                              (assert (= resolved.document
                                                         expected-syntax.document)
                                                      "component snapshot copied the derived document")
                                              (each [key data (pairs expected-syntax.captures)]
                                                (assert (= (. resolved.captures
                                                              key)
                                                           data)
                                                        "component snapshot copied derived captures"))
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
  (misa._commit))

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

(local view (misa.markdown_view.new_document))
(fn render []
  (local model (. db.messages.blocks 1))
  (local syntax (assert (misa.syntax_projection db model)))
  (set syntax.columns 80)
  (view:render (or model.text (table.concat model.chunks)) syntax))

(fn keyword? [lines]
  (accumulate [found false _ line (ipairs lines) &until found]
    (accumulate [found false _ span (ipairs line.spans) &until found]
      (accumulate [found false _ token (ipairs (if (= (type span.style) :string)
                                                   [span.style]
                                                   (or span.style [])))
                   &until found]
        (= token :syntax.keyword)))))

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
  (assert (= entry.document nil))
  (each [_ slot (ipairs entry.slots)]
    (assert (= slot.data nil) "capture arrays entered transactional db")))

(local reset-ok (pcall misa._dispatch {:type :transcript/reset :reject true}
                       terminal {:wall_ms 0 :monotonic_ms 0}))

(assert (not reset-ok))
(dispatch {:type :test/read})
(assert (keyword? (render)) "aborted reset destroyed accepted captures")
(dispatch {:type :ui/redraw})
(assert (= (length requests) 2) "projection or redraw scheduled syntax work")
(assert (= colored (render)) "unrelated transaction discarded cached rendering")
(set expected-syntax (misa.syntax_projection db (. db.messages.blocks 1)))
(set (. db.components.roles :transcript.assistant) :test.syntax-input)
(misa.transcript_projection db terminal)
(set (. db.components.roles :transcript.assistant) nil)
;; Real component model snapshots preserve the explicit input and require no
;; synchronous native capability during projection.
(assert (> (length (misa.transcript_projection db terminal)) 0))
(delta "\nlocal b")
(assert (= (length requests) 3))
(assert (not (keyword? (render))) "new source retained stale captures")
(dispatch {:type :transcript/reset})
(complete 3 true)
(assert (= (length requests) 3) "reset resurrected an old syntax request")
(start)
(delta "```lua\nlocal c")
(assert (= (length requests) 4))
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

(output "async syntax regressions passed\n")
