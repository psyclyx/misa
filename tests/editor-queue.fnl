(local fennel (require :fennel))
(local dofile fennel.dofile)

;; Exercise editor, modal policy and queue through real framework transactions.

(local output io.write)

(local context
       {:argv {}
        :config {:components {:persist false} :themes {:persist false}}})

(dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)

(local app ((require :tests.application) context))
(local declarations (require :misa.definitions))
(each [_ name (ipairs [:misa.json
                       :misa.keybindings
                       :misa.actions
                       :misa.ui.layout
                       :misa.commands
                       :misa.choices
                       :misa.ui.themes
                       :misa.ui.themes.default
                       :misa.ui.components
                       :misa.editor.render
                       :misa.agent
                       :misa.editor.queue
                       :misa.editor
                       :misa.editor.images
                       :misa.editor.editing])]
  (app.include (require name) context))

(var (db native) nil)

(app.define (declarations :editor-queue-1 [{:catalog :events  :value {:event :app/start :handler (fn [state]
                                       {:patch {:models
                                            {:entries [{:id :capture/model
                                                        :model :model
                                                        :provider :capture}]
                                             :selected :capture/model}}})}}]))

(app.define (declarations :editor-queue-2 [{:catalog :events  :value {:event :test/read :handler (fn [state] (set db state) nil)}}]))

(app.define (declarations :editor-queue-3 [{:catalog :events  :value {:event :test/attachment :handler (fn [state event]
                                       {:patch {:editor {:attachments (misa.replace event.attachments)}}})}}]))

(app.define (declarations :editor-queue-4 [{:catalog :events  :value {:event :test/exit-after-response :handler (fn [state]
                                       {:patch {:agent {:exit_after_response true}}})}}]))

(app.install context)

(fn dispatch [event]
  (set native {})
  (local pending [event])
  (var at 1)
  (while (. pending at)
    (local effects
           (misa._dispatch (. pending at)
                           {:columns 80 :interactive true :lines 24}
                           {:monotonic_ms 0 :wall_ms 0}))
    (misa._commit)
    (set at (+ at 1))
    (each [_ effect (ipairs effects)]
      (if (= effect.type :dispatch)
          (tset pending (+ (length pending) 1) effect.event)
          (tset native (+ (length native) 1) effect)))
    (assert (< at 200) "event loop"))
  (misa._dispatch {:type :test/read} {:columns 80 :interactive true :lines 24}
                  {:monotonic_ms 0 :wall_ms 0})
  (misa._commit)
  nil)

(fn input [kind text] (dispatch {: kind : text :type :terminal/input}) nil)

(fn has [kind]
  (each [_ effect (ipairs native)]
    (when (= effect.type kind) (lua "return effect")))
  nil)

(fn prompt [] (. db.agent.messages (length db.agent.messages) :content 1 :text))

(local first {:source {:data :Zmlyc3Q= :media_type :image/png :type :base64}
              :type :image})

(local second {:source {:data :c2Vjb25k :media_type :image/png :type :base64}
               :type :image})

(dispatch {:type :app/start})

(input :text "first request")

(input :enter)

(assert (and (and db.editor.busy (= db.editor.text ""))
             (= (prompt) "first request")))

(input :text "while busy")

(input :shift_enter)

(input :text "second line")

(assert (= db.editor.text "while busy\nsecond line")
        "typing/ShiftEnter while busy was discarded")

(dispatch {:attachments [first] :type :test/attachment})

(input :enter)

(assert (and (and (= db.queue.pending "while busy\nsecond line")
                  (= (length db.queue.attachments) 1))
             (= db.editor.text "")))

(input :text :another)

(input :enter)

(assert (= db.queue.pending "while busy\nsecond line\nanother")
        "consecutive submissions did not coalesce")

(input :text :draft)

(dispatch {:attachments [second] :type :test/attachment})

(input :alt :e)

(assert (and (= db.queue.pending "") (= (length db.queue.attachments) 0)))

(assert (and (= db.editor.text "while busy\nsecond line\nanother\ndraft")
             (= (length db.editor.attachments) 2))
        "take lost draft or attachments")

(assert (and (= (. db.editor.attachments 1 :source :data) first.source.data)
             (= (. db.editor.attachments 2 :source :data) second.source.data)))

(input :escape)

(input :text :0)

(input :text :x)

(assert (and (= db.editor.mode :normal) (= (db.editor.text:sub (- 4)) :raft))
        "Vim editing while busy did not work")

(input :alt_enter)

(assert (and (and (= db.editor.text "") (= db.agent.status :cancelling))
             (has :operation/cancel))
        "AltEnter did not steer in normal mode")

(local pending db.queue.pending)

;; Exiting a CLI-style response must wait if another prompt is scheduled.

(dispatch {:type :test/exit-after-response})

(dispatch {:id :agent-1 :message :Cancelled :type :agent/error})

(assert (and (and (not (has :app/quit)) (= db.agent.request_seq 2))
             (= (prompt) pending))
        "pending prompt lost at completion/exit boundary")

(assert (= (length (. db.agent.messages (length db.agent.messages) :content)) 3)
        "steer did not carry both images")

(input :text "keep editing")

(input :ctrl_c)

(assert (and (= db.editor.text "keep editing") (= db.agent.status :cancelling))
        "cancel destroyed in-progress draft")

(dispatch {:id :agent-2 :message :Cancelled :type :agent/error})

(assert (and (not (has :app/quit)) (= db.editor.text "keep editing"))
        "completion discarded an unsent live draft")

(dispatch {:attachments {} :replace true :text "" :type :editor/restore})

(dispatch {:exit true :type :agent/completed})

(assert (has :app/quit)
        "one-shot response never completed after queue and draft emptied")

(dispatch {:type :agent/reset})

(dispatch {:attachments [first] :replace true :text "" :type :editor/restore})

(input :enter)

(assert (= (. db.agent.messages 1 :content 1 :type) :image)
        "image-only editor submission failed")

(dispatch {:attachments {} :replace true :text :line :type :editor/restore})

(input :shift_enter)

(input :escape)

(input :text :u)

(assert (= db.editor.text :line)
        "ShiftEnter was missing from insert undo history")

(dispatch {:type :images/paste})

(dispatch {:attachments {}
           :replace true
           :text "wait for image"
           :type :editor/restore})

(input :enter)

(input :alt_enter)

(assert (and (= db.editor.text "wait for image") (= db.queue.pending ""))
        "submission overtook an outstanding image paste")

(dispatch {:attachments {} :replace true :text "" :type :editor/restore})

(dispatch {:exit true :type :agent/completed})

(assert (not (has :app/quit))
        "completion exited while image acquisition was outstanding")

(output "editor queue regressions passed\n")

nil
