(local fennel (require :fennel))
(local output io.write)
(local runtime-debug debug)
(local runtime-os os)

(fn fixture [order interactive]
  (set _G.debug runtime-debug)
  (set _G.os runtime-os)
  (fennel.dofile :src/lua_runtime/framework.fnl)
  (local app _G.misa)
  (local context {:argv [] :config {:components {:persist false} :themes {:persist false}}})
  (local construction ((require :tests.application) context))
  (each [_ name (ipairs [:json :keybindings :actions :layout :commands :choices
                         :themes :theme/default :components :component/editor
                         :component/picker :values :choices/preview :choices/layout :agent])]
    (construction.include (fennel.dofile (.. :extensions/ name :.fnl)) context))
  (construction.define
   {:subscriptions {:test/lifecycle {:inputs [[:db/path :test_lifecycle]]
                                    :compute (fn [inputs] (or (. inputs 1) {}))}}
    :services {:editor.lifecycle.test [:test/lifecycle]}})
  (each [_ name (ipairs order)]
    (local description ((fennel.dofile (.. :extensions/ name :.fnl)) context))
    (assert (= description.interceptors nil) "domain lifecycle declared middleware")
    (construction.define description))
  (each [_ name (ipairs [:editing :dialogs :picker])]
    (construction.include (fennel.dofile (.. :extensions/ name :.fnl)) context))
  (var db nil)
  (local requests [])
  (local submissions [])
  (var native [])
  (construction.define
   {:subscriptions {:test/other {:inputs [[:db/path :test_other]]
                                :compute (fn [inputs] (or (. inputs 1) {}))}}
    :services {:editor.lifecycle.other [:test/other]}
    :events {:test/read {:event :test/read :handler (fn [state] (set db state))}
             :test/state {:event :test/state :handler (fn [_ event] {:patch event.patch})}
             :test/submit {:event :agent/submit :handler (fn [_ event] (table.insert submissions event.prompt) nil)}
             :test/chosen {:event :test/chosen :handler (fn [_ event] {:patch {:chosen (app.replace event)}})}
             :test/start {:event :app/start
                          :handler (fn [] {:patch {:agent {:exit_after_response true}
                                                  :models {:selected :capture/model
                                                           :entries [{:id :capture/model :model :model :provider :capture}]}}})}}
    :commands {:/ping {:description :Ping :event :test/chosen}}
    :effects {:provider.capture (fn [effect] (table.insert requests effect) [])}})
  (assert (not (pcall construction.define {:services {:editor.lifecycle.other [:test/other]}}))
          "duplicate contributor was accepted")
  (construction.install)
  (local terminal {:columns 80 :lines 24 : interactive})
  (local clock {:wall_ms 0 :monotonic_ms 0})
  (fn read-state []
    (app._dispatch {:type :test/read} terminal clock)
    (app._commit)
    db)
  (fn step [event]
    (local before (app.json.encode event))
    (local fx (app._dispatch event terminal clock))
    (app._commit)
    (assert (= before (app.json.encode event)) "event was rewritten")
    (read-state)
    fx)
  (fn run [events]
    (set native [])
    (local pending (icollect [_ event (ipairs events)] event))
    (var at 1)
    (var quit false)
    (while (and (. pending at) (not quit))
      (each [_ effect (ipairs (step (. pending at)))]
        (if (= effect.type :dispatch) (table.insert pending effect.event)
            (do (table.insert native effect)
                (when (= effect.type :app/quit) (set quit true)))))
      (set at (+ at 1))
      (assert (< at 100) "lifecycle dispatch did not settle"))
    quit)
  (fn dispatch [event] (run [event]))
  (dispatch {:type :app/start})
  {: dispatch : run : step : requests : submissions :db (fn [] db) :app app
   :native (fn [] native)})

;; Both event orders, both terminal modes: drain before considering one-shot exit.
(each [_ order (ipairs [[:queue :images :editor] [:editor :images :queue]])]
  (each [_ interactive (ipairs [false true])]
    (local f (fixture order interactive))
    (f.dispatch {:type :queue/submit :prompt :first})
    (f.dispatch {:type :queue/submit :prompt :second})
    (assert (= (length f.requests) 1))
    (assert (not (f.dispatch {:type :agent/error :id :agent-1 :message :failed})))
    (assert (= (length f.requests) 2) "completion lost queued submission")
    (assert (= (. (f.db) :agent :status) :working))
    (assert (not (. (f.db) :queue :sending)))
    (assert (f.dispatch {:type :agent/error :id :agent-2 :message :failed})
            "empty one-shot did not quit")

    ;; A reserved submit can be behind a completion already in the native queue.
    (local reserved (fixture order interactive))
    (local fx (reserved.step {:type :queue/submit :prompt :reserved}))
    (assert (. (reserved.db) :queue :sending))
    (assert (= (. (reserved.db) :queue :pending) ""))
    (assert (not (reserved.run [{:type :agent/completed :exit true}
                               (. fx 1 :event) (. fx 2 :event)])))
    (assert (= (length reserved.requests) 1))
    (assert (not (. (reserved.db) :queue :sending)))
    (assert (reserved.dispatch {:type :agent/error :id :agent-1 :message :failed}))

    ;; Rejection has no working status event, but must release the reservation.
    (local rejected (fixture order interactive))
    (rejected.dispatch {:type :test/state :patch {:models {:selected rejected.app.delete}}})
    (assert (rejected.dispatch {:type :queue/submit :prompt :rejected}))
    (assert (not (. (rejected.db) :queue :sending)))
    (assert (= (length rejected.requests) 0))

    ;; A second payload queued behind a rejected reservation must also settle.
    (local rejecting (fixture order interactive))
    (rejecting.dispatch {:type :test/state :patch {:models {:selected rejecting.app.delete}}})
    (local reject-fx (rejecting.step {:type :queue/submit :prompt :first}))
    (assert (rejecting.run [{:type :queue/submit :prompt :second}
                           (. reject-fx 1 :event) (. reject-fx 2 :event)]))
    (assert (= (table.concat rejecting.submissions ",") "first,second")
            "rejection quit before the next reserved submission")
    (assert (= (. (rejecting.db) :queue :pending) ""))
    (assert (not (. (rejecting.db) :queue :sending)))
    (assert (= (length rejecting.requests) 0))

    ;; Auth defers the payload in agent state: don't clear the reservation early.
    (local auth (fixture order interactive))
    (auth.dispatch {:type :test/state :patch {:auth_startup {:ready false}}})
    (auth.dispatch {:type :queue/submit :prompt :deferred})
    (assert (. (auth.db) :queue :sending))
    (assert (not (auth.dispatch {:type :agent/completed :exit true})))
    (auth.dispatch {:type :test/state :patch {:auth_startup {:ready true}}})
    (auth.dispatch {:type :auth/startup-ready})
    (assert (= (length auth.requests) 1))
    (assert (not (. (auth.db) :queue :sending)))
    (assert (auth.dispatch {:type :agent/error :id :agent-1 :message :failed}))

    ;; Only an interactive unsent draft/attachment vetoes one-shot exit.
    (local draft (fixture order interactive))
    (draft.dispatch {:type :editor/restore :replace true :text :unsent})
    (assert (= (draft.dispatch {:type :agent/completed :exit true}) (not interactive)))
    (draft.dispatch {:type :editor/restore :replace true :text "" :attachments [{}]})
    (assert (= (draft.dispatch {:type :agent/completed :exit true}) (not interactive)))
    (assert (not (draft.dispatch {:type :agent/completed :exit false})))
    (when (not interactive)
      (assert (= (. (draft.native) 1 :type) :terminal/read)))))

;; An open data-only contributor works with neither queue nor images installed.
(local custom (fixture [:editor] true))
(custom.dispatch {:type :test/state :patch {:test_lifecycle {:block_draft true}
                                          :test_other {:hold_exit true}}})
(custom.dispatch {:type :editor/restore :text :draft})
(local previous (custom.db))
(custom.dispatch {:type :terminal/input :kind :enter})
(assert (= (. (custom.db) :editor) previous.editor))
(custom.dispatch {:type :editor/steer})
(assert (= (. (custom.db) :editor) previous.editor))
(assert (= (. (custom.db) :editing) previous.editing))
(assert (= (length custom.requests) 0))
(custom.dispatch {:type :editor/restore :replace true :text ""})
(assert (not (custom.dispatch {:type :agent/completed :exit true})))
(custom.dispatch {:type :test/state :patch {:test_other {:hold_exit false}}})
(assert (custom.dispatch {:type :agent/completed :exit true}) "block_draft also blocked exit")

;; Acquisition guards only draft submission. Modal and command Enter still work.
(local images (fixture [:images :editor :queue] true))
(images.dispatch {:type :images/paste})
(images.dispatch {:type :editor/restore :text :draft})
(images.dispatch {:type :test/state :patch {:editor {:selection_start 0 :selection_end 2}
                                          :editing {:anchor 0 :undo [{:text :before}]}}})
(local before (images.db))
(images.dispatch {:type :terminal/input :kind :enter})
(images.dispatch {:type :editor/steer})
(assert (= (. (images.db) :editor) before.editor))
(assert (= (. (images.db) :editing) before.editing))
(assert (= (length images.requests) 0))
(images.dispatch {:type :editor/restore :replace true :text "/ping"})
(images.dispatch {:type :terminal/input :kind :enter})
(assert (= (. (images.db) :chosen :type) :test/chosen) "image guard swallowed command Enter")
(images.dispatch {:type :dialog/open :id :dialog :correlation :one :completion :test/chosen
                  :actions [{:id :accept :primary true}]})
(images.dispatch {:type :terminal/input :kind :enter})
(assert (= (. (images.db) :chosen :action) :accept) "image guard swallowed dialog Enter")
(images.dispatch {:type :picker/open :id :picker :token :one :completion :test/chosen
                  :title :Choose :items [{:value :picked}]})
(images.dispatch {:type :terminal/input :kind :enter})
(assert (= (. (images.db) :chosen :value) :picked) "image guard swallowed picker Enter")

;; Already-owned queue payloads never wait for unrelated draft image acquisition.
(images.dispatch {:type :queue/submit :prompt :owned})
(assert (= (length images.requests) 1))
(assert (not (images.dispatch {:type :agent/error :id :agent-1 :message :failed})))
(assert (= (. (images.db) :editor :text) ""))
(images.dispatch {:type :images/loaded :id "image:1" :ok true
                  :data {:width 1 :height 1 :mime_type :image/png :data :encoded}})
(assert (= (length images.requests) 1) "image completion auto-submitted the draft")
(assert (= (length (. (images.db) :editor :attachments)) 1))
(images.dispatch {:type :terminal/input :kind :enter})
(assert (= (length images.requests) 2))
(assert (= (. images.requests 2 :messages 2 :content 1 :type) :image))

(output "editor lifecycle contracts passed\n")
