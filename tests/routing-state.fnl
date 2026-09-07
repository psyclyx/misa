(local fennel (require :fennel))
(local output io.write)
(local runtime-debug debug)
(local G (require :tests.generators))

(fn fixture [reverse]
  (set _G.debug runtime-debug)
  (fennel.dofile :src/lua_runtime/framework.fnl)
  (local app _G.misa)
  (local context {:argv [] :config {}})
  (local routes [])
  (each [_ name (ipairs [:json :keybindings :actions :layout :commands :choices :values
                         :choice_preview :choice_layout :dialogs :picker :omnipicker
                         :history :editor :editing :selection :messages :models])]
    (local specs ((. (fennel.dofile (.. :extensions/ name :.fnl)) :setup) context))
    (each [_ spec (ipairs specs.fx)]
      (assert (not= spec.type :register/interceptor))
      (if (= spec.type :register/event-route) (table.insert routes spec)
          (not= spec.type :register/event) (app._setup_effects {:fx [spec]}))))
  (app._setup_effects
   {:fx [{:type :register/action
          :value {:id :test.custom :label "Custom" :keys [:alt+z]
                  :event {:type :custom :payload {:value 1}}
                  :available (fn [db] db.allow_custom)}}]})
  (if reverse
      (for [i (length routes) 1 -1] (app._setup_effects {:fx [(. routes i)]}))
      (each [_ route (ipairs routes)] (app._setup_effects {:fx [route]})))
  (var observed nil)
  (var state nil)
  (each [_ name (ipairs [:terminal/input :actions/open :dialog/input :picker/input
                         :omnipicker/open :custom :history/previous :history/next
                         :history/search :editing/action :editing/interrupt :selection/action
                         :selection/open :messages/scroll :messages/toggle-verbose :model/picker-open])]
    (app._setup_effects
     {:fx [{:type :register/event : name
            :handler (fn [db event] (set observed event) (set state db) nil)}]}))
  (app._setup_effects
   {:fx [{:type :register/event :name :test/state :handler (fn [_ event] {:patch event.patch})}]})
  (app._seal context)
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

;; The route primitive is event-scoped, not terminal-specific middleware.
(set _G.debug runtime-debug)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app _G.misa)
(app._setup (fennel.dofile :extensions/json.fnl) {})
(app._setup (fennel.dofile :extensions/keybindings.fnl) {:config {}})
(local calls {})
(var observed nil)
(var saved nil)
(each [_ id (ipairs [:left :right])]
  (app._setup_effects
   {:fx [{:type :register/event-route
          :value {: id :event :test/source :priority 10 :context [:db/path id]
                  :resolve (fn [context event]
                             (tset calls id (+ (or (. calls id) 0) 1))
                             (if (= context :invalid) false
                                 {:type :test/target : context :payload event.payload}))}}
         {:type :register/keybinding
          :value {:context :test/collision :action id :default [:x]}}]}))
(assert (not (pcall app.keybinding_action :test/collision {:kind :text :text :x}))
        "keybinding collision used registration order")
(app._setup_effects
 {:fx [{:type :register/event-route
        :value {:id :fallback :event :test/source :priority 0 :context [:db]
                :resolve (fn [] (tset calls :fallback (+ (or calls.fallback 0) 1)) nil)}}
       {:type :register/event-route
        :value {:id :target-route :event :test/target :priority 0 :context [:db]
                :resolve (fn [] (error "routed event was recursively routed"))}}
       {:type :register/event :name :test/state :handler (fn [_ event] {:patch event.patch})}
       {:type :register/event :name :test/read :handler (fn [db] (set saved db) nil)}]})
(each [_ name (ipairs [:test/source :test/target :test/unrelated])]
  (app._setup_effects
   {:fx [{:type :register/event : name
          :handler (fn [_ event] (set observed event)
                     {:fx [{:type :dispatch :event {:type :test/one-effect}}]})}]}))
(assert (not (pcall app._setup_effects
                    {:fx [{:type :register/interceptor :value {:id :legacy :before (fn [tx] tx)}}]}))
        "legacy global hook is still accepted")
(assert (not (pcall app._setup_effects
                    {:fx [{:type :register/event-route
                           :value {:id :left :event :other :priority 0 :context [:db] :resolve (fn [])}}]}))
        "duplicate route id was accepted")
(app._seal {:argv [] :config {}})
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
