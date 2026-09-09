;; Theme resolution preserves reusable component output and Markdown body caches.
(local fennel (require :fennel))
(local output io.write)
(local context
       {:argv []
        :config {:themes {:persist false} :components {:persist false}}})

(local app ((require :tests.application) context))

(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local definitions (require :tests.declarations))
(local boundary (. (require :tests.stock) :misa.ui.components.group))
(local groups (. (require :tests.stock) :misa.transcript.groups))
(assert (. boundary.components :default.group.boundary))
(assert (= (. boundary.components :default.transcript.group_footer) nil)
        "generic boundaries must not install transcript policy")

(assert (. groups.components :default.transcript.group_footer))
(assert (= (. groups.components :default.group.boundary) nil)
        "transcript groups must compose the selected boundary implementation")

(each [_ name (ipairs [:misa.json
                       :misa.ui.themes
                       :misa.ui.themes.default
                       :misa.ui.components
                       :misa.actions
                       :misa.ui.layout
                       :misa.markdown
                       :misa.markdown.render
                       :misa.ui.values
                       :misa.ui.components.group
                       :misa.transcript.groups
                       :misa.transcript.render
                       :misa.ui.components.content
                       :misa.ui.components.truncation
                       :misa.transcript.tools
                       :misa.transcript.tools.render])]
  (app.define (. (require :tests.stock) name)))

(local cached {:surface :panel
               :cursor {:row 1 :byte 0}
               :lines [{:id :cached-line
                        :spans [{:text :abc
                                 :style :plain
                                 :link "https://example.test"
                                 :action :fixture
                                 :animation {:id :fixture
                                             :interval_ms 40
                                             :frames [{:text :abc
                                                       :style :highlight}
                                                      {:text :xyz}]}}
                                {:text :link
                                 :style :plain
                                 :link "https://link.test"}]}]})

(local effects [])
(each [_ spec (ipairs [{:id :first :ink :red :paper :blue :flash :green}
                       {:id :second :ink :cyan :paper :yellow :flash :magenta}])]
  (table.insert effects
                {:catalog :themes
                 :id spec.id
                 :value {:palette {:ink spec.ink
                                   :paper spec.paper
                                   :flash spec.flash}
                         :styles {:plain {:foreground :ink}
                                  :panel {:background :paper}
                                  :hover {:background :flash}
                                  :highlight {:foreground :flash}}}}))

(var received-model nil)
(var received-context nil)
(table.insert effects {:catalog :components
                       :id :default.cached
                       :value {:render (fn [model render-context]
                                         (set received-model model)
                                         (set received-context render-context)
                                         cached)}})

(var db nil)
(table.insert effects
              {:catalog :events
               :value {:event :test/read :handler (fn [state] (set db state))}})

(table.insert effects
              {:catalog :services
               :id :choices.pending
               :value (fn [db]
                        (and db.editor db.editor.choice db.editor.choice.combo))})
(app.define (definitions.collect :fixture effects))
(app.install)
(local semantic (misa.json.encode cached))
(misa._dispatch {:type :app/start} {:columns 80 :lines 24 :interactive true}
                {:wall_ms 0 :monotonic_ms 0})

(misa._commit)
(misa._dispatch {:type :test/read} {:columns 80 :lines 24 :interactive true}
                {:wall_ms 0 :monotonic_ms 0})

(misa._commit)
(local model {:nested {:value :original}})
(local render-context {:columns 8 :nested {:value :original}})
(local original-db db)
(set db (misa.themes.swap db :first))
(assert (= original-db.themes.active :default) "theme swap mutated prior state")
(assert (= original-db.components db.components)
        "theme swap copied component state")
(local swapped-component (misa.components.swap db :other :default.cached))
(assert (= db.components.roles.other nil) "component swap mutated prior state")
(assert (= swapped-component.components.roles.other :default.cached))
(assert (= swapped-component.themes db.themes)
        "component swap copied theme state")
(local first (misa.components.render db :cached model render-context))
(assert (= received-model model) "component boundary copied model input")
(assert (= received-context render-context)
        "component boundary copied context input")
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
(assert (= (. first.lines 1 :spans 3 :text) " "))
(assert (= first-span.link "https://example.test"))
(assert (= first-span.action :fixture))
;; Unchanged metadata and text-only frames remain shared immutable values.
(assert (= first.cursor cached.cursor))
(assert (= (. first-span.animation.frames 2)
           (. cached.lines 1 :spans 1 :animation :frames 2)))

(set db (misa.themes.swap db :second))
(set render-context.columns 10)
(local second (misa.components.render db :cached model render-context))
(assert (= (. second.lines 1 :spans 1 :style :foreground) :cyan))
(assert (= (. second.lines 1 :spans 1 :animation :frames 1 :style :foreground)
           :magenta))

(assert (= (. second.lines 1 :spans 3 :text) "   "))
(assert (= first-span.style.foreground :red)
        "later resolution changed previous frame")

(assert (= (misa.json.encode cached) semantic)
        "repeat resolution changed cached semantics")

;; Message surfaces wrap cached body spans; response metadata stays in the group.
(local message {:id :body
                :response_id :response
                :text :**body**
                :rail :rail.assistant})

(local message-context {:columns 40 :interactive true})
(local render (. (misa.components.lookup db :transcript.assistant) :render))
(local (raw-first first-cache) (render message message-context))
(local first-body (. raw-first.lines 1))
(assert (= (length raw-first.lines) 1))
(set message.started_wall_ms 3723000)
(local (raw-second second-cache) (render message message-context first-cache))
(assert (= (length raw-second.lines) 1)
        "message title accumulated in body cache")

(assert (not= (. raw-second.lines 1) first-body)
        "message wrapper reused mutable line")

(assert (= (. raw-second.lines 1 :spans) first-body.spans)
        "message wrapper deep-copied cached spans")

(local before (misa.json.encode raw-second))
(misa.components.render db :transcript.assistant message message-context)
(assert (= (misa.json.encode raw-second) before)
        "theme resolution changed previous message output")

(local raw-third (render message message-context second-cache))
(assert (= (length raw-third.lines) 1))
(assert (= (. raw-third.lines 1 :spans) first-body.spans)
        "resolved render invalidated cached body")

;; Transcript metadata works without status and receives semantic money/time.
(fn title-text [view]
  (table.concat (icollect [_ span (ipairs (. view.lines 1 :spans))]
                  span.text)))

(each [_ example (ipairs [[{:type :money :currency :USD :pending true}
                           "cost pending"]
                          [{:type :money
                            :currency :USD
                            :amount 0
                            :unknown true}
                           "cost ?"]
                          [{:type :money
                            :currency :USD
                            :amount 1
                            :estimated true
                            :unknown true}
                           "~$1.00 + ?"]
                          [{:type :money :currency :USD :amount 0.0123}
                           "$0.0123"]])]
  (local fact (. example 1))
  (local source (misa.patch message {:cost (misa.replace fact)}))
  (local before (misa.json.encode source))
  (assert (: (title-text (misa.components.render db :transcript.group_footer
                                                 source
                                                 {:columns 100
                                                  :interactive true}))
             :find (. example 2) 1 true))
  (assert (= before (misa.json.encode source))))

(assert (= (. (misa.values.render {:type :timestamp :value 0}) 1 :text)
           "00:00:00"))
(local original-value-render misa.values.render)
(local shared-metadata [{:text :custom-time}])
(set misa.values.render
     (fn [fact context]
       (if (= fact.type :timestamp)
           (do
             (assert (= fact.value 3723000)) shared-metadata)
           (original-value-render fact context))))

(assert (= (length (. (misa.components.render db :transcript.group_header
                                              message message-context)
                      :lines)) 0))
(assert (: (title-text (misa.components.render db :transcript.group_footer
                                               message message-context))
           :find :custom-time 1 true))
(assert (not (: (title-text (misa.components.render db :transcript.tool_call
                                                    {:name :test
                                                     :started_wall_ms 3723000}
                                                    message-context))
                :find :custom-time 1 true)))

(assert (= (. shared-metadata 1 :style) nil)
        "metadata renderer mutated shared spans")
(set misa.values.render original-value-render)
;; Every railed row is part of the continuous message surface, including borders.
(local decorated-message
       (misa.components.render (misa.themes.swap db :default)
                               :transcript.assistant
                               {:rail :rail.assistant
                                :text "```zig\nconst value = 1;\n```\n\n| name | value |\n| --- | --- |\n| x | 1 |"}
                               {:columns 60 :interactive true}))

(var (chrome body) (values 0 0))
(each [_ line (ipairs decorated-message.lines)]
  (if (= line.content false)
      (do
        (set chrome (+ chrome 1))
        (each [_ part (ipairs line.spans)]
          (assert part.style.background
                  "railed Markdown chrome lost its background")))
      (do
        (set body (+ body 1))
        (assert (. line.spans 1 :style :background)
                "Markdown content lost its background"))))

(assert (and (> chrome 2) (> body 1)))
;; Code fills the middle of each row, including its gutter, while the three
;; outer columns on both sides retain the enclosing message background.
(local code-db (misa.themes.swap db :default))
(local code-view
       (misa.components.render code-db :transcript.assistant
                               {:rail :rail.assistant
                                :text "```zig\none\ntwo\n```"}
                               {:columns 40 :interactive true}))

(assert (= (length code-view.lines) 2))
(local parent-background (. (misa.themes.style code-db :surface.assistant)
                            :background))
(local code-background
       (. (misa.themes.style code-db :surface.code) :background))
(assert (not= (misa.json.encode parent-background)
              (misa.json.encode code-background)))
(each [_ line (ipairs code-view.lines)]
  (var column 0)
  (each [_ part (ipairs line.spans)]
    (for [_ 1 (misa.layout.width part.text)]
      (set column (+ column 1))
      (assert (= (misa.json.encode part.style.background)
                 (misa.json.encode (if (or (<= column 3) (> column 37))
                                       parent-background code-background)))
              "code surface crossed its gutter or outer margins")))
  (assert (= column 40)))

;; Collapsed thinking displays actual source rather than a placeholder label.
(local thought
       "- Check the parser.\n- Compare both paths.\n- Keep the source intact.\n- Run the tests.\n- Inspect the output.")
(local compact-thought
       (misa.components.render (misa.themes.swap db :default)
                               :transcript.thinking_collapsed
                               {:rail :rail.thinking :text thought}
                               {:columns 60 :interactive true}))

(local thought-text (table.concat (icollect [_ line (ipairs compact-thought.lines)]
                                    (table.concat (icollect [_ part (ipairs line.spans)]
                                                    part.text)))
                                  "\n"))

(assert (thought-text:find "Check the parser." 1 true))
(assert (thought-text:find "2 lines hidden" 1 true))
(assert (not (thought-text:find "Thinking" 1 true)))
(assert (not (thought-text:find "summary" 1 true)))
(each [_ line (ipairs compact-thought.lines)]
  (assert (= (. line.spans 1 :text) "┃ "))
  (each [_ part (ipairs line.spans)] (assert part.style.background)))

(local user-view
       (misa.components.render (misa.themes.swap db :default) :transcript.user
                               {:rail :rail.user :text :hello}
                               {:columns 60 :interactive true}))

(assert (= (length user-view.lines) 1))
(assert (= (. user-view.lines 1 :spans 1 :text) "┃ "))
;; Hover changes only the resolved background and clears when the pointer leaves.
;; Reused semantic component output must remain untouched.
(fn hover [action link]
  (misa._dispatch {:type :ui/hover : action : link}
                  {:columns 80 :lines 24 :interactive true}
                  {:wall_ms 0 :monotonic_ms 0})
  (misa._commit)
  (misa._dispatch {:type :test/read} {:columns 80 :lines 24 :interactive true}
                  {:wall_ms 0 :monotonic_ms 0})
  (misa._commit)
  (set db (misa.themes.swap db :first))
  (misa.components.render db :cached model render-context))

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

(local link-hovered (hover "" "https://link.test"))
(assert (not= (. resting.lines 1 :spans 2 :style :background)
              (. link-hovered.lines 1 :spans 2 :style :background))
        "OSC-only link did not receive hover feedback")

(assert (= (. resting.lines 1 :spans 1 :style :background)
           (. link-hovered.lines 1 :spans 1 :style :background)))

(local link-departed (hover ""))
(assert (= (misa.json.encode (. resting.lines 1 :spans 2 :style))
           (misa.json.encode (. link-departed.lines 1 :spans 2 :style))))

(assert (= (misa.json.encode cached) semantic))
(local pending-db (misa.patch db {:editor {:choice {:combo "alt+;"}}}))
(local disabled
       (misa.components.render pending-db :cached model render-context))
(assert (= (. disabled.lines 1 :spans 1 :action) nil)
        "pending combo left a button actionable")
(local restored (misa.components.render db :cached model render-context))
(assert (= (. restored.lines 1 :spans 1 :action) :fixture)
        "aborted combo did not restore buttons")
;; Resting hints are already dim: pending needs a distinct color, and stale
;; hover/animation styles must not make a disabled hint look active again.
(local default-db (misa.themes.swap db :default))
(local hint-component
       {:lines [{:spans [{:text "Alt-A"
                          :style :keybinding
                          :action :fixture
                          :animation {:frames [{:style :accent}]}}
                         {:text "Alt-O"
                          :style :bold
                          :action :fixture
                          :sequence_progress true}]}]})

(local enabled-hint
       (misa.components.resolve default-db hint-component render-context))
(local waiting-db
       (misa.patch default-db
                   {:hover_action :fixture :editor {:choice {:combo "alt+o"}}}))
(local waiting-hint
       (misa.components.resolve waiting-db hint-component render-context))
(local hint-style (. waiting-hint.lines 1 :spans 1 :style))
(assert hint-style.dim)
(assert (not= (misa.json.encode (. enabled-hint.lines 1 :spans 1 :style
                                   :foreground))
              (misa.json.encode hint-style.foreground)))

(assert (= hint-style.background nil) "disabled hint retained hover background")
(assert (= (misa.json.encode hint-style)
           (misa.json.encode (. waiting-hint.lines 1 :spans 1 :animation
                                :frames 1 :style)))
        "animation restored enabled hint styling")

(assert (. waiting-hint.lines 1 :spans 2 :style :bold)
        "pending style hid combo progress")
(local collection [{:id :button :role :cached : model}])
(local normal-view (misa.components.project db :combo-test collection
                                            render-context))
(local pending-view
       (misa.components.project pending-db :combo-test collection
                                render-context))
(assert (= (. pending-view.views 1 :lines 1 :spans 1 :action) nil)
        "component cache retained enabled hints")
(local resumed-view
       (misa.components.project db :combo-test collection render-context))
(assert (= (. resumed-view.views 1 :lines 1 :spans 1 :action) :fixture))
(output "component resolution regressions passed\n")
