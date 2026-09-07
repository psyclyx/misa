;; Drives real framework transactions, including input routing and rollback

;; cloning, rather than mutating a fake UI model between assertions.

{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/command
                         :value {:completion :fixture-values
                                 :description "Choice action fixture"
                                 :event :test/fixture
                                 :name :/fixture}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :fixture-values
                         :value {:value :one}})
          (table.insert setup-fx
                        {:type :register/completion
                         :group :fixture-values
                         :value {:value :two}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:event {:type :test/custom}
                                 :id :test.custom
                                 :keys [:alt+z]
                                 :label "Custom action"}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :test/custom
                         :handler (fn [db]
                                    {:patch {:custom_action true} :fx [{:type :terminal/read}]})})
          (local steps {})

          (fn step [event check]
            (tset steps (+ (length steps) 1) {: check : event})
            nil)

          (fn input [kind text] {: kind : text :type :terminal/input})

          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (set tx.cofx.terminal
                                                {:columns 60
                                                 :interactive true
                                                 :lines 24})
                                           tx)
                                 :id :interaction/terminal}})
          (step {:key :alt+z :kind :key :type :terminal/input}
                (fn [db]
                  (assert db.custom_action
                          "registered action key did not dispatch")
                  nil))
          (step (input :text "hello 界\nsecond line")
                (fn [db] (assert (= db.editor.text "hello 界\nsecond line"))
                  nil))
          (step (input :escape) (fn [db]
                                  (assert (= db.editor.mode :normal))
                                  (assert (= db.editor.cursor
                                             (- (length db.editor.text) 1)))
                                  nil))
          (step (input :text :0) (fn [db]
                                   (assert (= db.editor.cursor (length "hello 界
")))
                                   nil))
          (step (input :text :k) (fn [db] (assert (= db.editor.cursor 0)) nil))
          (step (input :text :w) (fn [db] (assert (= db.editor.cursor 6)) nil))
          (step (input :text :v))
          (step (input :text :y) (fn [db] (assert (= db.clipboard.text "界"))
                                   (assert (= db.editor.mode :normal))
                                   nil))
          (step (input :text :x) (fn [db]
                                   (assert (= db.editor.text
                                              "hello \nsecond line"))
                                   nil))
          (step (input :text :u) (fn [db]
                                   (assert (= db.editor.text "hello 界
second line"))
                                   nil))
          (step (input :text :U) (fn [db]
                                   (assert (= db.editor.text
                                              "hello \nsecond line")
                                           "U did not redo")
                                   nil))
          (step (input :text :u))
          (step (input :text :d))
          (step (input :text :w) (fn [db]
                                   (assert (= db.editor.text
                                              "hello second line"))
                                   nil))
          (step (input :text :u))
          (step (input :text :i))
          (step (input :text "!"))
          (step (input :escape))
          (step (input :text :u) (fn [db]
                                   (assert (= db.editor.text "hello 界
second line")
                                           "insert undo group was stale")
                                   nil))
          (step (input :f1) (fn [db]
                              (assert (and db.picker (= db.picker.id :actions))
                                      "F1 swallowed by normal mode")
                              nil))
          (step (input :escape) (fn [db] (assert (not db.picker)) nil))
          (step {:kind :ctrl_c :type :terminal/input}
                (fn [db] (assert (= db.editor.text "")) nil))
          (step (input :text ":transcript")
                (fn [db]
                  (assert (and db.picker
                               (= db.picker.session.query :transcript)))
                  nil))
          (step (input :escape))
          (step (input :text " :") (fn [db]
                                     (assert (= db.editor.text " :")
                                             "leading whitespace invoked actions")
                                     nil))
          (step (input :ctrl_c))
          (step (input :text "one\ntwo"))
          (step (input :escape))
          (step (input :text :y))
          (step (input :text :y) (fn [db]
                                   (assert (and db.clipboard.linewise
                                                (= db.clipboard.text :two)))
                                   nil))
          (step (input :text :p) (fn [db]
                                   (assert (= db.editor.text "one\ntwo\ntwo")
                                           "linewise paste inserted inside line")
                                   nil))
          (step (input :ctrl_c))
          (step {:completion :test/picked
                 :id :nested-fixture
                 :items [{:value "item one"}
                         {:value "item two"}
                         {:value "item three"}]
                 :title :Fixture
                 :token :nested-fixture
                 :type :picker/open
                 :views [:all]})
          (step (input :text :item))
          (step (input :arrow_down) (fn [db]
                                      (assert (= (. db.picker.session.panels 1
                                                    :highlight)
                                                 2))
                                      nil))
          (step (input :f1) (fn [db]
                              (assert (and (= db.picker.id :actions)
                                           (= db.picker.parent.id
                                              :nested-fixture))
                                      "F1 did not nest over picker")
                              (var found false)
                              (each [_ item (ipairs db.picker.session.items)]
                                (when (= item.id :choices.next)
                                  (set found true)))
                              (assert found
                                      "choice actions missing from key reference")
                              nil))
          (step (input :f1) (fn [db]
                              (assert (and (= db.picker.id :actions)
                                           (= db.picker.parent.id
                                              :nested-fixture))
                                      "F1 nested an action palette inside itself")
                              nil))
          (step (input :escape) (fn [db]
                                  (assert (and (and (= db.picker.id
                                                       :nested-fixture)
                                                    (= db.picker.session.query
                                                       :item))
                                               (= (. db.picker.session.panels 1
                                                     :highlight)
                                                  2))
                                          "cancel did not restore picker query/highlight")
                                  nil))
          (step (input :f1))
          (step (input :text :choices.next))
          (step (input :enter))
          (step {:type :test/idle} (fn [db]
                                     (assert (and (= db.picker.id
                                                     :nested-fixture)
                                                  (= (. db.picker.session.panels
                                                        1 :highlight)
                                                     3))
                                             "choice action did not execute on restored parent")
                                     nil))
          (step (input :f1))
          (step (input :text :commands.open))
          (step (input :enter))
          (step {:type :test/idle} (fn [db]
                                     (assert (and (= db.picker.id :omnipicker)
                                                  (= db.picker.parent.id
                                                     :nested-fixture))
                                             "global command action did not nest over original picker")
                                     nil))
          (step (input :escape) (fn [db]
                                  (assert (and (and (= db.picker.id
                                                       :nested-fixture)
                                                    (= db.picker.session.query
                                                       :item))
                                               (= (. db.picker.session.panels 1
                                                     :highlight)
                                                  3))
                                          "global nested picker cancellation lost original state")
                                  nil))
          (step (input :escape) (fn [db] (assert (not db.picker)) nil))
          (step (input :text "/fixture ")
                (fn [db]
                  (assert (and db.editor.choice
                               (= (. db.editor.choice.panels 1 :highlight) 1)))
                  nil))
          (step (input :f1))
          (step (input :text :choices.next))
          (step (input :enter))
          (step {:type :test/idle} (fn [db]
                                     (assert (and (not db.picker)
                                                  (= (. db.editor.choice.panels
                                                        1 :highlight)
                                                     2))
                                             "choice action did not route to inline session")
                                     nil))
          (step (input :ctrl_c))
          (step {:content [{:text "# Heading

A paragraph.

| Name | Value |
| --- | --- |
| one | 界 |"
                            :type :text}]
                 :request_id :selection-fixture
                 :type :transcript/assistant})
          (step {:type :selection/open}
                (fn [db]
                  (assert (= (length db.selection.documents) 1))
                  nil))
          (step (input :text :l))
          ;; message -> section
          (step (input :text :l))
          ;; section -> heading
          (step (input :text :j))
          ;; content
          (step (input :text :l))
          ;; paragraph
          (step (input :text :y) (fn [db]
                                   (assert (= db.clipboard.text "A paragraph.")
                                           "paragraph source changed")
                                   nil))
          (step (input :text :j))
          ;; table
          (step (input :text :l))
          ;; header row
          (step (input :text :j))
          ;; data row
          (step (input :text :l))
          ;; name cell
          (step (input :text :j))
          ;; value cell
          (step (input :text :y) (fn [db]
                                   (assert (= db.clipboard.text "界")
                                           "table cell copy changed source")
                                   nil))
          (step (input :f1) (fn [db]
                              (assert (and db.picker (= db.picker.id :actions))
                                      "F1 swallowed by structural selection")
                              nil))
          (step (input :escape))
          (step (input :escape) (fn [db] (assert (not db.selection)) nil))
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:event {:index 1
                                                   :type :interaction/step}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :interaction/step
                         :handler (fn [db event]
                                    (local current (. steps event.index))
                                    (if (not current)
                                        {:fx [{:lines [{:spans [{:text :interaction}]}]
                                               :type :view/commit}
                                              {:type :app/quit}]}
                                        {:fx [{:event current.event
                                               :type :dispatch}
                                              {:event {:index event.index
                                                       :type :interaction/check}
                                               :type :dispatch}]}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :interaction/check
                         :handler (fn [db event]
                                    ;; Follow-up effects (clipboard/copy, picker/open) are queued after this
                                    ;; transaction; an extra event places the check behind those effects.
                                    {:fx [{:event {:index event.index
                                                   :type :interaction/assert}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :interaction/assert
                         :handler (fn [db event]
                                    (local check (. steps event.index :check))
                                    (when check
                                      (local (ok err) (pcall check db))
                                      (assert ok
                                              (.. "interaction step "
                                                  event.index ": "
                                                  (tostring err))))
                                    {:fx [{:event {:index (+ event.index 1)
                                                   :type :interaction/step}
                                           :type :dispatch}]})})
          nil
          {:fx setup-fx})}
