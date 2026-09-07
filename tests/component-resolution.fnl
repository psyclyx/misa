;; Theme resolution preserves reusable component output and Markdown body caches.
(local fennel (require :fennel))
(local output io.write)
(local context
       {:argv []
        :config {:themes {:persist false} :components {:persist false}}})

(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(each [_ name (ipairs [:json
                       :themes
                       :theme/default
                       :components
                       :actions
                       :layout
                       :markdown
                       :component/markdown
                       :component/message])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))

(local cached
       {:surface :panel
        :cursor {:row 1 :byte 0}
        :lines [{:id :cached-line
                 :spans [{:text :abc
                          :style :plain
                          :link "https://example.test"
                          :action :fixture
                          :animation {:id :fixture
                                      :interval_ms 40
                                      :frames [{:text :abc :style :highlight}
                                               {:text :xyz}]}}]}]})

(local semantic (misa.json.encode cached))
(local effects [])
(each [_ spec (ipairs [{:id :first :ink :red :paper :blue :flash :green}
                       {:id :second :ink :cyan :paper :yellow :flash :magenta}])]
  (table.insert effects
                {:type :register/theme
                 :id spec.id
                 :value {:palette {:ink spec.ink
                                   :paper spec.paper
                                   :flash spec.flash}
                         :styles {:plain {:foreground :ink}
                                  :panel {:background :paper}
                                  :hover {:background :flash}
                                  :highlight {:foreground :flash}}}}))

(table.insert effects {:type :register/component
                       :id :default.cached
                       :value {:render (fn [model render-context]
                                         (set model.nested.value :changed)
                                         (set render-context.nested.value
                                              :changed)
                                         cached)}})

(var db nil)
(table.insert effects
              {:type :register/event
               :name :test/read
               :handler (fn [state] (set db state))})

(misa._setup_effects {:fx effects})
(misa._seal context)
(misa._dispatch {:type :app/start} {:columns 80 :lines 24 :interactive true}
                {:wall_ms 0 :monotonic_ms 0})

(misa._commit)
(misa._dispatch {:type :test/read} {:columns 80 :lines 24 :interactive true}
                {:wall_ms 0 :monotonic_ms 0})

(misa._commit)
(local model {:nested {:value :original}})
(local render-context {:columns 8 :nested {:value :original}})
(local original-db db)
(set db (misa.swap_theme db :first))
(assert (= original-db.themes.active :default) "theme swap mutated prior state")
(assert (= original-db.components db.components) "theme swap copied component state")
(local swapped-component (misa.swap_component db :other :default.cached))
(assert (= db.components.roles.other nil) "component swap mutated prior state")
(assert (= swapped-component.components.roles.other :default.cached))
(assert (= swapped-component.themes db.themes) "component swap copied theme state")
(local first (misa.render_component db :cached model render-context))
(assert (= model.nested.value :original) "component mutated caller model")
(assert (= render-context.nested.value :original)
        "component mutated caller context")
(assert (= (misa.json.encode cached) semantic)
        "theme resolution changed cached semantics")
(assert (and (not= first cached) (not= first.lines cached.lines)
             (not= (. first.lines 1) (. cached.lines 1)))
        "resolved line records alias cache")

(local first-span (. first.lines 1 :spans 1))
(assert (= first-span.style.foreground :red))
(assert (= first-span.style.background :blue))
(assert (= (. first-span.animation.frames 1 :style :foreground) :green))
(assert (= (. first-span.animation.frames 1 :style :background) :blue))
(assert (= (. first.lines 1 :spans 2 :text) "     "))
(assert (= first-span.link "https://example.test"))
(assert (= first-span.action :fixture))
;; Unchanged metadata and text-only frames remain shared immutable values.
(assert (= first.cursor cached.cursor))
(assert (= (. first-span.animation.frames 2)
           (. cached.lines 1 :spans 1 :animation :frames 2)))
(set db (misa.swap_theme db :second))
(set render-context.columns 10)
(local second (misa.render_component db :cached model render-context))
(assert (= (. second.lines 1 :spans 1 :style :foreground) :cyan))
(assert (= (. second.lines 1 :spans 1 :animation :frames 1 :style :foreground)
           :magenta))
(assert (= (. second.lines 1 :spans 2 :text) "       "))
(assert (= first-span.style.foreground :red)
        "later resolution changed previous frame")
(assert (= (misa.json.encode cached) semantic)
        "repeat resolution changed cached semantics")

;; Message chrome adds titles/surfaces without copying or mutating cached spans.
(local message {:id :body
                :response_id :response
                :text :**body**
                :rail :rail.assistant})

(local message-context {:columns 40 :interactive true})
(local render (. (misa.component db :transcript.assistant) :render))
(local raw-first (render message message-context))
(local first-body (. raw-first.lines 2))
(assert (= (length raw-first.lines) 2))
(set message.timestamp :later)
(local raw-second (render message message-context))
(assert (= (length raw-second.lines) 2)
        "message title accumulated in body cache")
(assert (not= (. raw-second.lines 2) first-body)
        "message wrapper reused mutable line")
(assert (= (. raw-second.lines 2 :spans) first-body.spans)
        "message wrapper deep-copied cached spans")
(assert (= (length (. raw-first.lines 1 :spans)) 1)
        "new title changed previous title")
(local before (misa.json.encode raw-second))
(misa.render_component db :transcript.assistant message message-context)
(assert (= (misa.json.encode raw-second) before)
        "theme resolution changed previous message output")
(local raw-third (render message message-context))
(assert (= (length raw-third.lines) 2))
(assert (= (. raw-third.lines 2 :spans) first-body.spans)
        "resolved render invalidated cached body")
;; Hover changes only the resolved background and clears when the pointer leaves.
;; Reused semantic component output must remain untouched.
(fn hover [action]
  (misa._dispatch {:type :ui/hover : action}
                  {:columns 80 :lines 24 :interactive true}
                  {:wall_ms 0 :monotonic_ms 0})
  (misa._commit)
  (misa._dispatch {:type :test/read}
                  {:columns 80 :lines 24 :interactive true}
                  {:wall_ms 0 :monotonic_ms 0})
  (misa._commit)
  (set db (misa.swap_theme db :first))
  (misa.render_component db :cached model render-context))
(local resting (hover ""))
(local hovered (hover :fixture))
(local departed (hover ""))
(assert (not= (. resting.lines 1 :spans 1 :style :background)
              (. hovered.lines 1 :spans 1 :style :background))
        "hover did not change button background")
(assert (= (misa.json.encode (. resting.lines 1 :spans 1 :style))
           (misa.json.encode (. departed.lines 1 :spans 1 :style)))
        "leaving a button retained hover styling")
(assert (= (misa.json.encode cached) semantic)
        "hover mutated reusable component data")
(output "component resolution regressions passed\n")
