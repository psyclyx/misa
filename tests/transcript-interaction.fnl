(local definitions (require :misa.definitions))

;; Integration coverage over the real message, Markdown, selection, and choice

;; services. Assertions inspect the rich transcript itself, never a mock view.

(fn []
          (local declarations [])
          (table.insert declarations
                        (let [definition {:id :fixture/model
                                 :model :model
                                 :pricing {:input 2 :output 8}
                                 :provider :fixture}] {:catalog :models :id (. definition :id) :value definition}))
          (local context {:columns 54 :images true :interactive true})
          (local steps {})
          (var (original anchor) nil)

          (fn step [event check]
            (tset steps (+ (length steps) 1) {: check : event})
            nil)

          (fn text [lines]
            (local result {})
            (each [_ line (ipairs lines)]
              (local parts {})
              (each [_ span (ipairs (or line.spans {}))]
                (tset parts (+ (length parts) 1) (or span.text "")))
              (tset result (+ (length result) 1) (table.concat parts)))
            (table.concat result "\n"))

          (fn transcript [db] (misa.transcript.project db context))

          (fn selected-text [db]
            (local result {})
            (local style (misa.themes.style db :selection))

            (fn same [a b]
              (or (= a b) (and (and (and (and (= (type a) :table)
                                              (= (type b) :table))
                                         (= a.r b.r))
                                    (= a.g b.g))
                               (= a.b b.b))))

            (each [_ line (ipairs (transcript db))]
              (each [_ span (ipairs (or line.spans {}))]
                (when (and span.style
                           (same span.style.background style.background))
                  (tset result (+ (length result) 1) span.text))))
            (table.concat result))

          (fn selection [action check]
            (step {: action :type :selection/action} check)
            nil)

          (step {:content [{:text "First **boldword** and [linkword](https://example.com).

Second paragraph with useful words."
                            :type :text}]
                 :request_id :rich
                 :type :transcript/assistant})
          (step {:content [{:text (string.rep "Filler transcript line.\n\n" 14)
                            :type :text}]
                 :request_id :tail
                 :type :transcript/assistant}
                (fn [db]
                  (local lines (transcript db))
                  (var (bold link) (values false false))
                  (each [_ line (ipairs lines)]
                    (each [_ span (ipairs line.spans)]
                      (when (= span.text :boldword)
                        (set bold (= span.style.bold true)))
                      (when (= span.text :linkword)
                        (set link (= span.link "https://example.com")))))
                  (assert (and bold link)
                          "fixture did not render rich Markdown")
                  (set original (text lines))
                  (misa.transcript.window db context 6)
                  nil))
          (step {:type :selection/open})
          (selection :first (fn [db]
                              (assert (= (text (transcript db)) original)
                                      "message selection replaced or reflowed the existing transcript")
                              (var dock false)
                              (each [_ layer (ipairs (misa.ui.layers db
                                                                       {:available_lines 20
                                                                        :terminal {:columns 54
                                                                                   :lines 24}}))]
                                (assert (and (not layer.overlay)
                                             (not layer.exclusive))
                                        "selection opened a replacement overlay")
                                (set dock (or dock (= layer.dock :input))))
                              (assert dock
                                      "selection lost its input-area controls")
                              (local visible
                                     (misa.transcript.window db context 6))
                              (assert (: (text visible) :find :First 1 true)
                                      "selection did not reveal the selected message")
                              (assert (= (text (misa.transcript.window db
                                                                       context 6))
                                         (text visible))
                                      "same-selection redraw jumped away from its revealed message")
                              nil))
          (selection :child (fn [db]
                              (assert (= (. (misa.selection.state db)
                                            :kind)
                                         :paragraph))
                              (local highlighted (selected-text db))
                              (assert (and (and (highlighted:find :First 1 true)
                                                (highlighted:find :boldword 1
                                                                  true))
                                           (not (highlighted:find :Second 1
                                                                  true)))
                                      "paragraph selection highlighted unrelated source")
                              (var (bold link) (values false false))
                              (each [_ line (ipairs (transcript db))]
                                (each [_ span (ipairs line.spans)]
                                  (when (= span.text :boldword)
                                    (set bold (= span.style.bold true)))
                                  (when (= span.text :linkword)
                                    (set link
                                         (= span.link "https://example.com")))))
                              (assert (and bold link)
                                      "selection stripped existing emphasis or hyperlink metadata")
                              nil))
          (selection :child)
          ;; paragraph -> source line
          (selection :child)
          ;; source line -> First
          (selection :next (fn [db]
                             (assert (= (. (misa.selection.state db) :kind)
                                        :word))
                             (assert (= (selected-text db) :boldword)
                                     (.. "word selection did not decorate exactly the rendered source word: "
                                         (selected-text db)))
                             nil))
          (selection :copy)
          (step {:type :test/transcript-idle}
                (fn [db]
                  (assert (= db.clipboard.text :**boldword**)
                          "word copy did not preserve original Markdown bytes")
                  nil))
          (selection :close (fn [db]
                              (assert (= (text (transcript db)) original)
                                      "leaving selection changed transcript content")
                              (misa.transcript.window db context 6)
                              nil))
          (step {:delta 10 :type :messages/scroll}
                (fn [db]
                  (set anchor (text (misa.transcript.window db context 6)))
                  (assert (and db.messages.top (> db.messages.scroll 0))
                          "scroll did not move off the live transcript tail")
                  (assert (= (text (misa.transcript.window db context 6))
                             anchor)
                          "idle redraw lost scroll anchor")
                  nil))
          (step {:model :fixture/model
                 :response_id :live
                 :type :transcript/response-start})
          (step {:block_id :live/thinking
                 :kind :thinking
                 :response_id :live
                 :type :transcript/block-start})
          (step {:block_id :live/thinking
                 :response_id :live
                 :text "Visible private rationale"
                 :type :transcript/block-delta}
                (fn [db]
                  (local rendered (text (transcript db)))
                  (assert (and (not db.messages.verbose)
                               (rendered:find "Visible private rationale" 1
                                              true))
                          "active thinking was collapsed in summary mode")
                  (assert (rendered:find :streaming 1 true)
                          "active transcript block had no streaming marker")
                  (assert (= (text (misa.transcript.window db context 6))
                             anchor)
                          "streaming new blocks displaced the scroll anchor")
                  nil))
          (step {:block_id :live/text
                 :kind :assistant
                 :response_id :live
                 :type :transcript/block-start})
          (step {:block_id :live/text
                 :response_id :live
                 :text "Pending answer"
                 :type :transcript/block-delta}
                (fn [db]
                  (local rendered (text (transcript db)))
                  (assert (and (rendered:find "Pending answer" 1 true)
                               (rendered:find :streaming 1 true)))
                  (assert (= (text (misa.transcript.window db context 6))
                             anchor)
                          "assistant deltas displaced the scroll anchor")
                  nil))
          (step {:model :fixture/model
                 :response_id :live
                 :type :transcript/response-end
                 :usage {:cost_usd 0.0123 :input_tokens 20 :output_tokens 10}}
                (fn [db]
                  (local rendered (text (transcript db)))
                  (assert (rendered:find :$0.0123 1 true)
                          "completed message did not show response cost")
                  (assert (not (rendered:find :streaming 1 true))
                          "completed response retained streaming marker")
                  (assert (rendered:find "Visible private rationale" 1 true)
                          "finished thinking did not retain an actual text preview")
                  (local service misa.costs.response)
                  (set misa.costs.response nil)
                  (assert (pcall transcript db)
                          "transcript rendering required optional costs plugin")
                  (set misa.costs.response service)
                  nil))
          (step {:attachments [{:height 1
                                :image_id 77
                                :name :fixture.png
                                :preview {:data :AAAA/w== :height 1 :width 1}
                                :type :image
                                :width 1}]
                 :text "Attached image"
                 :type :transcript/user}
                (fn [db]
                  (local lines (transcript db))
                  (var image false)
                  (each [_ line (ipairs lines)]
                    (when line.image
                      (set image
                           (and (= line.image.id 77)
                                (= line.image.format :rgba)))))
                  (assert (and image (: (text lines) :find :fixture.png 1 true))
                          "submitted image attachment lines or image payload were lost")
                  (assert (= (text (misa.transcript.window db context 6))
                             anchor)
                          "new user message displaced anchored transcript")
                  nil))
          ;; Supply native terminal facts before dispatch, never by rewriting cofx.
          (local dispatch misa._dispatch)
          (set misa._dispatch
               (fn [event _ clock]
                 (dispatch event {:columns 54 :interactive true :lines 24 :images true} clock)))
          (local project misa._project)
          (set misa._project
               (fn [_ clock]
                 (project {:columns 54 :interactive true :lines 24 :images true} clock)))
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {
                                     :fx [{:event {:index 1
                                                   :type :test/transcript-step}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/transcript-step :handler (fn [db event]
                                    (local item (. steps event.index))
                                    (if (not item)
                                        {
                                         :fx [{:lines [{:spans [{:text "transcript interaction"}]}]
                                               :type :view/commit}
                                              {:type :app/quit}]}
                                        {
                                         :fx [{:event item.event
                                               :type :dispatch}
                                              {:event {:index event.index
                                                       :type :test/transcript-wait}
                                               :type :dispatch}]}))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/transcript-wait :handler (fn [db event]
                                    {
                                     :fx [{:event {:index event.index
                                                   :type :test/transcript-check}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/transcript-check :handler (fn [db event]
                                    (when (. steps event.index :check)
                                      ((. steps event.index :check) db))
                                    {
                                     :fx [{:event {:index (+ event.index 1)
                                                   :type :test/transcript-step}
                                           :type :dispatch}]})}})
          nil
          (definitions.build :tests.transcript-interaction declarations {}))
