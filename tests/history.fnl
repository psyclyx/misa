(local definitions (require :misa.definitions))

(fn [context]
          (local declarations [])
          (local steps {})

          (fn step [event check]
            (tset steps (+ (length steps) 1) {: check : event})
            nil)

          ;; The editor adapter is deliberately tiny: this fixture tests history's
          ;; event contract without loading the Vim policy or transcript presentation.
          (table.insert declarations
                        {:catalog :events  :value {:event :editor/restore :handler (fn [db event]
                                    (assert (= event.replace true))
                                    {:patch {:editor {:text event.text
                                                      :cursor (or event.cursor (length event.text))
                                                      :attachments (misa.replace event.attachments)}}})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/editor :handler (fn [db event]
                                    {:patch {:editor (misa.replace
                                         {:attachments (or event.attachments {})
                                          :cursor (or event.cursor
                                                      (length event.text))
                                          :text event.text})}})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :terminal/input :handler (fn [db]
                                    {:patch {:unhandled (+ (or db.unhandled 0) 1)}})}})
          (step {:prompt :one :type :agent/submitted})
          (step {:prompt :one :type :agent/submitted}
                (fn [db]
                  (assert (= (length db.history.entries) 1)
                          "consecutive history was not deduplicated")
                  nil))
          (step {:prompt "two\nsecond line" :type :agent/submitted})
          (step {:prompt :one :type :agent/submitted}
                (fn [db]
                  (assert (= (length db.history.entries) 3)
                          "nonconsecutive history disappeared")
                  nil))
          (step {:attachments [{:path :image.png}]
                 :cursor 2
                 :text "draft\nlast"
                 :type :test/editor})
          (step {:kind :arrow_up :type :terminal/input}
                (fn [db]
                  (assert (and (= db.editor.text :one)
                               (= (length db.editor.attachments) 0)))
                  nil))
          (step {:kind :ctrl_p :type :terminal/input}
                (fn [db] (assert (= db.editor.text "two\nsecond line")) nil))
          (step {:kind :ctrl_n :type :terminal/input}
                (fn [db] (assert (= db.editor.text :one)) nil))
          (step {:kind :arrow_down :type :terminal/input}
                (fn [db]
                  (assert (and (and (= db.editor.text "draft\nlast")
                                    (= db.editor.cursor 2))
                               (= (. db.editor.attachments 1 :path) :image.png))
                          "cycling history lost the original draft")
                  nil))
          (step {:kind :arrow_down :type :terminal/input}
                (fn [db]
                  (assert (= db.unhandled 1)
                          "Down stole multiline cursor movement")
                  nil))
          (step {:cursor 8 :text "first\nlast" :type :test/editor})
          (step {:kind :arrow_up :type :terminal/input}
                (fn [db]
                  (assert (= db.unhandled 2)
                          "Up stole multiline cursor movement")
                  nil))
          (step {:key :alt+p :kind :key :type :terminal/input}
                (fn [db]
                  (assert (= db.editor.text :one)
                          "Alt-P was restricted by cursor line")
                  nil))
          (step {:key :alt+n :kind :key :type :terminal/input}
                (fn [db]
                  (assert (and (= db.editor.text "first\nlast")
                               (= db.editor.cursor 8)))
                  nil))
          (step {:kind :ctrl_r :type :terminal/input}
                (fn [db]
                  (assert (and (= db.picker.id :input-history)
                               (= (length db.picker.session.items) 2))
                          "search did not globally deduplicate")
                  (assert (= (. db.picker.session.items 1 :value) :one)
                          "search was not ordered newest first")
                  nil))
          (step {:kind :text :text :second :type :picker/input})
          (step {:kind :enter :type :picker/input}
                (fn [db]
                  (assert (and (not db.picker) (= db.editor.text "two
second line"))
                          "search did not restore exact multiline input")
                  nil))
          (local config (or context.config.history {}))
          (local limit (or config.max_entries 500))
          (local loaded {})
          (for [index 1 (+ limit 2)]
            (tset loaded (+ (length loaded) 1) (.. "loaded " index)))
          (step {:data {:entries loaded :version 1}
                 :found true
                 :namespace :history
                 :type :history/loaded}
                (fn [db]
                  (assert (and (<= (length db.history.entries) limit)
                               (= (. db.history.entries
                                     (length db.history.entries))
                                  :one))
                          "history loading lost recent submissions or exceeded its entry bound")
                  {:history_test_count (length db.history.entries)}))
          (step {:prompt (string.rep :x (+ (or config.max_bytes 65536) 1))
                 :type :agent/submitted}
                (fn [db]
                  (assert (= (length db.history.entries) db.history_test_count)
                          "oversized input displaced bounded history")
                  nil))
          (table.insert declarations
                        {:catalog :events  :value {:event :app/start :handler (fn [db]
                                    {:patch {:editor (misa.replace {:cursor 0 :text ""})}
                                     :fx [{:event {:index 1
                                                   :type :test/history-step}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/history-step :handler (fn [db event]
                                    (local item (. steps event.index))
                                    (if (not item)
                                        {:fx [{:lines [{:spans [{:text :history}]}]
                                               :type :view/commit}
                                              {:type :app/quit}]}
                                        {:fx [{:event item.event
                                               :type :dispatch}
                                              {:event {:index event.index
                                                       :type :test/history-wait}
                                               :type :dispatch}]}))}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/history-wait :handler (fn [db event]
                                    {:fx [{:event {:index event.index
                                                   :type :test/history-wait-again}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/history-wait-again :handler (fn [db event]
                                    {:fx [{:event {:index event.index
                                                   :type :test/history-check}
                                           :type :dispatch}]})}})
          (table.insert declarations
                        {:catalog :events  :value {:event :test/history-check :handler (fn [db event]
                                    (local check (. steps event.index :check))
                                    {:patch (and check (check db))
                                     :fx [{:event {:index (+ event.index 1)
                                                   :type :test/history-step}
                                           :type :dispatch}]})}})
          nil
          (definitions :tests.history declarations {}))
