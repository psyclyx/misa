;; Conversation storage and projection preserve full text and Markdown styling,
;; independently of tool-preview budgets.
(local fennel (require :fennel))
(local output io.write)
(local context {:config {:themes {:persist false}
                         :components {:persist false}
                         :messages {:max_string 4000}}
                :argv []})

(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) context))
(local declarations (require :misa.definitions))
(each [_ name (ipairs [:misa.keybindings
                       :misa.ui.themes
                       :misa.ui.themes.default
                       :misa.ui.components
                       :misa.ui.layout
                       :misa.markdown
                       :misa.markdown.render
                       :misa.ui.values :misa.ui.components.content :misa.ui.components.truncation :misa.transcript.tools :misa.transcript.tools.render
                       :misa.ui.components.group :misa.transcript.groups :misa.transcript.render
                       :misa.transcript])]
  (app.include (require name) context))

(var db nil)
(app.define (declarations :long-messages-1 [{:catalog :events  :value {:event :test/read :handler (fn [state] (set db state))}}]))

(app.install context)
(local terminal {:interactive true :columns 80 :lines 24})
(fn dispatch [event]
  (local pending [event])
  (var at 1)
  (while (. pending at)
    (local effects
           (misa._dispatch (. pending at) terminal {:wall_ms 0 :monotonic_ms 0}))
    (misa._commit)
    (each [_ effect (ipairs effects)]
      (when (= effect.type :dispatch) (table.insert pending effect.event)))
    (set at (+ at 1)))
  (misa._dispatch {:type :test/read} terminal {:wall_ms 0 :monotonic_ms 0})
  (misa._commit))

(fn text-of [lines]
  (local text [])
  (each [_ line (ipairs lines)]
    (each [_ span (ipairs line.spans)] (table.insert text span.text)))
  (table.concat text))

(fn visible [marker]
  (assert (: (text-of (misa.transcript.project db terminal)) :find marker 1
             true) (.. "message tail hidden: " marker)))

(dispatch {:type :app/start})
(local first (.. (string.rep :abcdefghij 600) " FIRST-END"))
(local tail " SECOND-END")
(dispatch {:type :transcript/response-start :response_id :long})
(dispatch {:type :transcript/block-start
           :response_id :long
           :block_id :body
           :kind :assistant})

(dispatch {:type :transcript/block-delta
           :response_id :long
           :block_id :body
           :text first})

(visible :FIRST-END)
(dispatch {:type :transcript/block-delta
           :response_id :long
           :block_id :body
           :text tail})

(visible :SECOND-END)
(dispatch {:type :transcript/response-end :response_id :long})
(assert (= (. db.messages.blocks 1 :text) (.. first tail)))
(assert (not (. db.messages.blocks 1 :truncated)))
(each [_ verbose (ipairs [false true])]
  (when (not= db.messages.verbose verbose)
    (dispatch {:type :messages/toggle-verbose}))
  (visible :FIRST-END)
  (visible :SECOND-END))

(assert (= (text-of (misa.markdown.view.plain (.. first tail))) (.. first tail)))
(dispatch {:type :transcript/user :text (.. first " USER-END")})
(assert (= (. db.messages.blocks 2 :text) (.. first " USER-END")))
(visible :USER-END)
(dispatch {:type :transcript/assistant
           :request_id :legacy
           :content [{:type :text :text (.. first " LEGACY-END")}]})

(assert (= (. db.messages.blocks 3 :text) (.. first " LEGACY-END")))
(visible :LEGACY-END)
;; A hidden transcript must not invoke its potentially expensive projection.
(let [project misa.transcript.project]
  (set misa.transcript.project
       (fn [] (error "hidden transcript attempted projection")))
  (each [_ room (ipairs [0 -1])]
    (assert (= (length (misa.transcript.window db terminal room)) 0)))
  (set misa.transcript.project project))

(assert (> (length (misa.transcript.window db terminal 1)) 0)
        "visible transcript did not resume projection")

;; Formatting continues beyond the former document-size and block-count caps.
(fn has-strong [lines marker]
  (accumulate [found false _ line (ipairs lines) &until found]
    (accumulate [matched false _ span (ipairs line.spans) &until matched]
      (and (= span.text marker)
           (accumulate [strong false _ mark (ipairs (or span.style []))
                        &until strong]
             (= mark :bold))))))

(local large (.. (string.rep "ordinary text\n\n" 18000) :**BYTE-TAIL**))
(assert (> (length large) 262144))
(assert (has-strong (. (misa.markdown.view.project large {:columns 80}) :lines) :BYTE-TAIL)
        "long source lost Markdown styling")

(local many-blocks (.. (string.rep "# heading\n\n" 5000) :**BLOCK-TAIL**))
(assert (has-strong (. (misa.markdown.view.project many-blocks {:columns 80}) :lines) :BLOCK-TAIL)
        "block count hid or flattened Markdown")

;; Tool arguments keep their independent budget and structural redaction.
(dispatch {:type :transcript/tool-call
           :id :tool
           :name :demo
           :arguments {:command first :token :secret}})

(local tool (. db.messages.blocks 4))
(assert (: tool.arguments.command :find "[truncated" 1 true))
(assert (= tool.arguments.token "[redacted]"))
(dispatch {:type :transcript/tool-result :id :tool :text first})
(assert (= (. db.messages.blocks 4 :result) first))
(local result-text (.. (string.rep (.. (string.rep :x 60) "\n") 100) "RESULT-END"))
(dispatch {:type :transcript/tool-result :id :tool :text result-text})
(assert (= (. db.messages.blocks 4 :result) result-text))
(when db.messages.verbose (dispatch {:type :messages/toggle-verbose}))
(local collapsed-text (text-of (misa.transcript.project db terminal)))
(assert (collapsed-text:find "98 lines hidden" 1 true) "result truncation did not count all retained rows")
(assert (not (collapsed-text:find "RESULT-END" 1 true)))
(dispatch {:type :messages/toggle-verbose})
(visible "RESULT-END")
;; Standalone results obey the same retention and sanitization contract.
(dispatch {:type :transcript/tool-result :id :standalone :text (.. "\27[31m" result-text "\27[0m")})
(assert (= (. db.messages.blocks 5 :text) result-text))
(dispatch {:type :transcript/tool-result :id :standalone :text (.. "\27[31m" result-text "\27[0m")})
(assert (= (. db.messages.blocks 5 :text) result-text))
(assert (= (. db.messages.blocks 5 :result) result-text))
(output "long message regressions passed\n")
