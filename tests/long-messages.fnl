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
(each [_ name (ipairs [:keybindings
                       :themes
                       :theme/default
                       :components
                       :layout
                       :markdown
                       :component/markdown
                       :values :component/tool
                       :component/message
                       :messages])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))

(var db nil)
(misa._setup_effects {:fx [{:type :register/event
                            :name :test/read
                            :handler (fn [state] (set db state))}]})

(misa._seal context)
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
  (assert (: (text-of (misa.transcript_projection db terminal)) :find marker 1
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

(assert (= (text-of (misa.markdown_view.plain (.. first tail))) (.. first tail)))
(dispatch {:type :transcript/user :text (.. first " USER-END")})
(assert (= (. db.messages.blocks 2 :text) (.. first " USER-END")))
(visible :USER-END)
(dispatch {:type :transcript/assistant
           :request_id :legacy
           :content [{:type :text :text (.. first " LEGACY-END")}]})

(assert (= (. db.messages.blocks 3 :text) (.. first " LEGACY-END")))
(visible :LEGACY-END)
;; A hidden transcript must not invoke its potentially expensive projection.
(let [project misa.transcript_projection]
  (set misa.transcript_projection
       (fn [] (error "hidden transcript attempted projection")))
  (each [_ room (ipairs [0 -1])]
    (assert (= (length (misa.transcript_window db terminal room)) 0)))
  (set misa.transcript_projection project))

(assert (> (length (misa.transcript_window db terminal 1)) 0)
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
(assert (has-strong (. (misa.markdown_view.project large {:columns 80}) :lines) :BYTE-TAIL)
        "long source lost Markdown styling")

(local many-blocks (.. (string.rep "# heading\n\n" 5000) :**BLOCK-TAIL**))
(assert (has-strong (. (misa.markdown_view.project many-blocks {:columns 80}) :lines) :BLOCK-TAIL)
        "block count hid or flattened Markdown")

;; Tool previews keep their independent limit and structural redaction.
(dispatch {:type :transcript/tool-call
           :id :tool
           :name :demo
           :arguments {:command first :token :secret}})

(local tool (. db.messages.blocks 4))
(assert (: tool.arguments.command :find "[truncated" 1 true))
(assert (= tool.arguments.token "[redacted]"))
(dispatch {:type :transcript/tool-result :id :tool :text first})
(assert (: (. db.messages.blocks 4 :result) :find "[truncated" 1 true))
(output "long message regressions passed\n")
