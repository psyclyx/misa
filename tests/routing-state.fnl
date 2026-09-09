(local fennel (require :fennel))
(local output io.write)
(local runtime-debug debug)
(local runtime-os os)
(local G (require :tests.generators))

(fn fixture [reverse]
  (set _G.debug runtime-debug)
  (set _G.os runtime-os)
  (fennel.dofile :src/lua_runtime/framework.fnl)
  (local app _G.misa)
  (local context {:argv [] :config {}})
  (local construction ((require :tests.application) context))
  (local routes [])
  (each [_ name (ipairs [:json :keybindings :actions :layout :commands :choices :values
                         :choices/preview :choices/layout :dialogs :picker :omnipicker
                         :history :editor :editing :selection :messages :models])]
    (local description ((fennel.dofile (.. :extensions/ name :.fnl)) context))
    (assert (= description.interceptors nil))
    (each [id value (pairs (or description.routes {}))] (table.insert routes {: id : value}))
    (construction.define (collect [kind entries (pairs description)]
                           (when (and (not= kind :routes) (not= kind :events)) (values kind entries)))))
  (construction.define {:actions {:test.custom {:label "Custom" :keys [:alt+z]
                                                :event {:type :custom :payload {:value 1}}
                                                :available (fn [db] db.allow_custom)}}})
  (fn add-route [entry] (construction.define {:routes {entry.id entry.value}}))
  (if reverse (for [i (length routes) 1 -1] (add-route (. routes i)))
      (each [_ route (ipairs routes)] (add-route route)))
  (var observed nil)
  (var state nil)
  (each [_ name (ipairs [:terminal/input :actions/open :dialog/input :picker/input
                         :choices/dispatch :choices/ignored :omnipicker/open :custom :history/previous :history/next
                         :history/search :editing/action :editing/interrupt :selection/action
                         :selection/open :messages/scroll :messages/toggle-verbose :model/picker-open])]
    (construction.define {:events {name {:event name :handler (fn [db event] (set observed event) (set state db) nil)}}}))
  (construction.define {:events {:test/state {:event :test/state :handler (fn [_ event] {:patch event.patch})}}})
  (construction.install)
  (fn dispatch [event]
    (set _G.misa app)
    (local fx (app._dispatch event {:columns 80 :lines 24 :interactive true}
                            {:wall_ms 0 :monotonic_ms 0}))
    (app._commit)
    fx)
  (fn route [db event]
    (dispatch {:type :test/state
               :patch {:editor (app.replace db.editor)
                       :dialog (if db.dialog (app.replace db.dialog) app.delete)
                       :picker (if db.picker (app.replace db.picker) app.delete)
                       :selection (if db.selection (app.replace db.selection) app.delete)
                       :allow_custom (or db.allow_custom false)}})
    (local before (app.json.encode {: db : event}))
    (set observed nil)
    (local effects (dispatch event))
    (assert (= (length effects) 0) "routing emitted extra effects")
    (assert (= before (app.json.encode {: db : event})) "routing mutated input")
    (assert observed "input disappeared")
    (values observed state))
  {: route :app app})

(local forward (fixture false))
(local backward (fixture true))
(local initial {:editor {:text "" :cursor 0 :mode :insert}})
(fn check [db event expected]
  (local a (forward.route db event))
  (local b (backward.route db event))
  (assert (= a.type expected) (.. "unexpected route: " a.type " expected " expected))
  (assert (= (forward.app.json.encode a) (backward.app.json.encode b))
          "route registration order changed dispatch")
  a)
(fn input [kind text] {:type :terminal/input : kind : text})
(local dialog {:editor initial.editor :dialog {:id :test} :picker {:id :test}})
(local picker {:editor initial.editor :picker {:id :test}})
(local selected {:editor initial.editor :selection {:id :test}})
(check initial (input :arrow_up) :history/previous)
(check initial (input :arrow_down) :history/next)
(check initial (input :text ":find") :actions/open)
(check dialog {:type :terminal/input :kind :key :key :f1} :dialog/input)
(check dialog (input :text :x) :dialog/input)
(check picker {:type :terminal/input :kind :key :key :f1} :actions/open)
(check picker (input :text :x) :picker/input)
(check picker (input :alt :m) :picker/input)
(check initial (input :alt :M) :model/picker-open)
(check initial (input :alt "/") :omnipicker/open)
(check initial (input :escape) :editing/action)
(check initial (input :alt :z) :terminal/input)
(check {:editor initial.editor :allow_custom true} (input :alt :z) :custom)
(check selected (input :text :x) :selection/action)
(check selected (input :wheel_up) :messages/scroll)
(check selected (input :page_down) :messages/scroll)
(check {:editor {:text "one\ntwo" :cursor 5 :mode :insert}} (input :arrow_up) :terminal/input)
(check {:editor {:text "one\ntwo" :cursor 1 :mode :insert}} (input :arrow_down) :terminal/input)
(check {:editor {:text "draft" :cursor 0 :mode :normal}} (input :ctrl_c) :editing/interrupt)
(check {:editor {:text "draft" :cursor 0 :mode :normal}} (input :ctrl_d) :editing/interrupt)
(check {:editor {:text "draft" :cursor 0 :mode :normal}} (input :eof) :editing/interrupt)

(local pending {:editor {:text "/model " :cursor 7 :mode :insert :choice {:combo "alt+;"}}})
(each [_ event (ipairs [(input :alt :m) (input :alt :f) (input :text ":")
                       (input :escape) (input :backspace) (input :ctrl_c)
                       {:type :terminal/input :kind :key :key :f1}])]
  (check pending event :terminal/input)
  (check {:editor initial.editor :picker {:session {:combo "alt+;"}}} event :picker/input))

(check picker {:type :ui/action :action :choices.option_1_10} :choices/dispatch)
(check {:editor initial.editor :picker {:session {:combo "alt+o"}}}
       {:type :ui/action :action :choices.option_1_10} :choices/ignored)

(local failure
       (G.for_all (G.tuple [(G.elements [initial dialog picker selected])
                            (G.elements [:text :alt :key :arrow_up :arrow_down :escape :wheel_up])
                            (G.elements [:x :P ":" "" "hello"])])
                  (fn [sample]
                    (local event (input (. sample 2) (. sample 3)))
                    (local a (forward.route (. sample 1) event))
                    (local b (backward.route (. sample 1) event))
                    (assert (= (forward.app.json.encode a) (backward.app.json.encode b))))
                  {:cases 500}))
(assert (not failure) (and failure (fennel.view failure)))

;; Routes inspect only their named source event.
(set _G.debug runtime-debug)
(set _G.os runtime-os)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app _G.misa)
(local construction ((require :tests.application) {:argv [] :config {}}))
(construction.include (fennel.dofile :extensions/json.fnl) {})
(construction.include (fennel.dofile :extensions/keybindings.fnl) {:config {}})
(local calls {})
(var observed nil)
(var saved nil)
(each [_ id (ipairs [:left :right])]
  (construction.define
   {:routes {id {:event :test/source :priority 10 :context [:db/path id]
                 :resolve (fn [context event]
                            (tset calls id (+ (or (. calls id) 0) 1))
                            (if (= context :invalid) false
                                {:type :test/target : context :payload event.payload}))}}
    :keybindings {id {:context :test/collision :action id :default [:x]}}}))
(construction.define
 {:routes {:fallback {:event :test/source :priority 0 :context [:db]
                      :resolve (fn [] (tset calls :fallback (+ (or calls.fallback 0) 1)) nil)}
           :target-route {:event :test/target :priority 0 :context [:db]
                          :resolve (fn [] (error "routed event was recursively routed"))}}
  :events {:test/state {:event :test/state :handler (fn [_ event] {:patch event.patch})}
           :test/read {:event :test/read :handler (fn [db] (set saved db) nil)}}})
(each [_ name (ipairs [:test/source :test/target :test/unrelated])]
  (construction.define
   {:events {name {:event name :handler (fn [_ event] (set observed event)
                                         {:fx [{:type :dispatch :event {:type :test/one-effect}}]})}}}))
(assert (= app.reg_interceptor nil) "legacy global hook is still exposed")
(assert (not (pcall construction.define
                    {:routes {:left {:event :other :priority 0 :context [:db] :resolve (fn [])}}}))
        "duplicate route ID was accepted")
(construction.install)
(assert (not (pcall app.keybindings.action :test/collision {:kind :text :text :x}))
        "keybinding collision used declaration order")
(fn step [event]
  (local fx (app._dispatch event {:columns 80 :lines 24 :interactive false}
                          {:wall_ms 0 :monotonic_ms 0}))
  (app._commit)
  fx)
(step {:type :test/unrelated})
(assert (= (next calls) nil) "route inspected an unrelated event")
(step {:type :test/source})
(assert (= observed.type :test/source))
(assert (= calls.fallback 1))
(step {:type :test/state :patch {:left false}})
(local payload {:value :shared})
(local effects (step {:type :test/source : payload}))
(assert (= observed.type :test/target))
(assert (= observed.context false) "false context was treated as inactive")
(assert (= observed.payload payload))
(assert (= (length effects) 1) "routing duplicated execution effects")
(assert (= calls.fallback 1) "lower-priority route ran after a claim")
(step {:type :test/state :patch {:right true}})
(step {:type :test/read})
(local before saved)
(assert (not (pcall step {:type :test/source})) "ambiguous winning routes were accepted")
(step {:type :test/read})
(assert (= before saved) "failed routing changed committed state")
(step {:type :test/state :patch {:right app.delete :left :invalid}})
(assert (not (pcall step {:type :test/source})) "invalid route result was accepted")
(output "routing transaction ownership passed\n")
