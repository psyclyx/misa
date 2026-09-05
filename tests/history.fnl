{:setup (fn [context]
          (local setup-fx [])
          (local steps {})

          (fn step [event check]
            (tset steps (+ (length steps) 1) {: check : event})
            nil)

          ;; The editor adapter is deliberately tiny: this fixture tests history's
          ;; event contract without loading the Vim policy or transcript presentation.
          (table.insert setup-fx
                        {:type :register/event
                         :name :editor/restore
                         :handler (fn [db event]
                                    (assert (= event.replace true))
                                    (set db.editor.text event.text)
                                    (set db.editor.cursor
                                         (or event.cursor (length event.text)))
                                    (set db.editor.attachments
                                         event.attachments)
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/editor
                         :handler (fn [db event]
                                    (set db.editor
                                         {:attachments (or event.attachments {})
                                          :cursor (or event.cursor
                                                      (length event.text))
                                          :text event.text})
                                    {: db})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :terminal/input
                         :handler (fn [db]
                                    (set db.unhandled (+ (or db.unhandled 0) 1))
                                    {: db})})
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
                  (set db.history_test_count (length db.history.entries))
                  nil))
          (step {:prompt (string.rep :x (+ (or config.max_bytes 65536) 1))
                 :type :agent/submitted}
                (fn [db]
                  (assert (= (length db.history.entries) db.history_test_count)
                          "oversized input displaced bounded history")
                  nil))
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (set db.editor {:cursor 0 :text ""})
                                    {: db
                                     :fx [{:event {:index 1
                                                   :type :test/history-step}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/history-step
                         :handler (fn [db event]
                                    (local item (. steps event.index))
                                    (if (not item)
                                        {: db
                                         :fx [{:lines [{:spans [{:text :history}]}]
                                               :type :view/commit}
                                              {:type :app/quit}]}
                                        {: db
                                         :fx [{:event item.event
                                               :type :dispatch}
                                              {:event {:index event.index
                                                       :type :test/history-wait}
                                               :type :dispatch}]}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/history-wait
                         :handler (fn [db event]
                                    {: db
                                     :fx [{:event {:index event.index
                                                   :type :test/history-wait-again}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/history-wait-again
                         :handler (fn [db event]
                                    {: db
                                     :fx [{:event {:index event.index
                                                   :type :test/history-check}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/history-check
                         :handler (fn [db event]
                                    (when (. steps event.index :check)
                                      ((. steps event.index :check) db))
                                    {: db
                                     :fx [{:event {:index (+ event.index 1)
                                                   :type :test/history-step}
                                           :type :dispatch}]})})
          nil
          {:fx setup-fx})}
