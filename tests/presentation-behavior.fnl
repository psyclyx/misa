(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local markdown (require :misa.markdown))
(local model (require :misa.transcript.model))
(local viewport (require :misa.transcript.viewport))
(local styles (require :misa.ui.themes.styles))
(local validation (require :misa.ui.components.validation))

;; These APIs run without installing event handlers, services, or a view.
(local first (markdown.parse "# Heading\n\nparagraph\n\n```diff\n-old\n+new"))
(local second (markdown.parse "# Heading\n\nparagraph\n\n```diff\n-old\n+new\n```"
                              first))

(assert (= (. first.blocks 1) (. second.blocks 1))
        "unchanged Markdown prefix lost identity")

(assert (= second (markdown.parse second.source second))
        "unchanged document was reallocated")

(assert (= (. second.blocks (length second.blocks) :text) "-old\n+new"))
(assert (= (. (markdown.parse "a | b\n--- | --\nx | y") :blocks 1 :kind)
           :paragraph) "invalid table delimiter was accepted")

(local source {:palette {:accent "#abcdef"}
               :styles {:plain {:foreground :default}
                        :accent {:foreground :accent}}})

(local theme (styles.normalize source
                               {:palette {:accent "#123456"}
                                :styles {:accent {:bold true}}}))

(local projected (styles.compose theme [:plain :accent]))
(assert (= projected.foreground.r 18))
(assert projected.bold)
(set projected.foreground.r 0)
(assert (= theme.styles.accent.foreground.r 18)
        "style result shares mutable colors")

(assert (= source.palette.accent "#abcdef") "normalization mutated source data")
(assert (not (pcall styles.normalize source {:styles {:accent {:bogus true}}})))

(local view {:lines [{:spans [{:text "safe"}]}]})
(assert (= view (validation.view view)) "validation changed cache identity")
(assert (not (pcall validation.view {:lines [{:spans [{:text "bad\nline"}]}]})))
(assert (not (pcall validation.view
                    {:lines [{:spans [{:text "é"}]}] :cursor {:row 1 :byte 1}})))

(fn apply [db transaction] (misa.patch db transaction.patch))
(local policy {:max_depth 8 :max_items 64 :max_string 4000 :redact {}})
(local clock {:clock {:wall_ms 100 :monotonic_ms 10}})
(local initial (apply {} (model.initialize {:verbose true} {})))
(local started
       (apply initial (model.start-response initial {:response_id :r} clock)))

(local streaming
       (apply started
              (model.start-block started
                                 {:response_id :r
                                  :block_id :b
                                  :kind :assistant})))

(local delta {:response_id :r :block_id :b :text "hello"})
(local updated (apply streaming
                      (model.update-block {:assistant model.text-delta} policy
                                          streaming delta)))

(assert (= (length (. streaming.messages.blocks 1 :chunks)) 0)
        "stream update mutated retained state")

(assert (= (. updated.messages.blocks 1 :chunks 1) "hello"))
(local finished
       (apply updated
              (model.finish-streaming-block policy updated
                                            {:response_id :r :block_id :b})))

(assert (= (. finished.messages.blocks 1 :text) "hello"))
(assert (not (pcall model.update-block {:assistant model.text-delta} policy
                    finished delta)) "finalized block accepted a delta")

(local rows
       (icollect [index (ipairs [:a :b :c :d :e])]
         {:transcript_id :one :source_start (* index 10) :spans []}))

(local geometry {:first 4 :total 5 :room 2 :layout rows :selection {:id :one}})
(local scrolled (apply {:messages {}} (viewport.scroll geometry 2)))
(assert (= scrolled.messages.top 2))
(assert (= scrolled.messages.anchor.source 20))
(assert (= scrolled.messages.scroll 2))
(local bottom (apply scrolled (viewport.scroll geometry -10)))
(assert (= bottom.messages.top nil))
(assert (= bottom.messages.anchor nil))
(assert (= bottom.messages.scroll 0))
(assert (= (. (viewport.scroll nil 1) :patch) nil))
(local animation-state (require :misa.ui.animations.state))
(local catalog {:moving {:frames ["a" "b"]} :still {:frames ["x"]}})
(local options {:enabled true :interval_ms 160})
(local idle {:animations {:active :moving :roles {} :running {} :ticks {}}})
(local start (animation-state.start catalog options idle :status))
(assert (= (. start.fx 1 :type) :timer/start))
(local running (apply idle start))
(assert (= (. (animation-state.reconcile catalog options running) :fx 1) nil)
        "unchanged timer state restarted the timer")

(local tick
       (animation-state.tick catalog options running
                             {:id :animation/service :count 99}))

(assert (= (. tick.patch :animations :ticks :status) 1)
        "timer event count leaked into the transactional animation clock")

(assert (= (animation-state.tick catalog options running {:id :unrelated}) nil))
(assert (= (. (animation-state.stop catalog options running :status) :fx 1
              :type) :timer/stop))

(assert (= (. (animation-state.start catalog {:enabled false :interval_ms 160}
                                     idle :status) :fx 1) nil)
        "disabled animation started a timer")

(local indicators (require :misa.ui.status.indicators))
(local selection-config
       [{:id :model :priority 7}
        {:id :alpha :hotkey true}
        {:id :zebra :representation :icon}])

(local chosen (indicators.selections selection-config))
(assert (= (length chosen) 3))
(assert (= (. chosen 1 :id) :model))
(assert (= (. chosen 1 :priority) 7))
(assert (= (. chosen 2 :id) :alpha))
(assert (= (. chosen 3 :id) :zebra))
(assert (= (. selection-config 1 :representation) nil)
        "selection normalization mutated configuration")

(assert (not (pcall indicators.selections [:model :model])))
(output "presentation behavior contracts passed\n")
