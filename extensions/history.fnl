;; Submitted input history. Navigation and search are policies over ordinary

;; editor restore events and generic choice sessions; drafts are never stored.

(fn available [db]
  (and (and (and (and (not= db.editor nil) (not db.picker)) (not db.dialog))
            (not db.selection)) (not db.editor.choice)))

(fn state [db]
  (set db.history (or db.history {:entries {} :index 0 :sequence 0}))
  db.history)

(fn restore [text attachments cursor]
  {:event {:attachments (or attachments {})
           : cursor
           :replace true
           : text
           :type :editor/restore}
   :type :dispatch})

(fn add [entries text]
  (if (and (and (= (type text) :string) (not= text ""))
           (not= (. entries (length entries)) text))
      (do
        (tset entries (+ (length entries) 1) text)
        true) false))

{:setup (fn [context]
          (var config (or (and (= (type context.config) :table)
                               context.config.history)
                          {}))
          (set config (or (and (= (type config) :table) config) {}))
          (local (max-entries max-bytes)
                 (values (or config.max_entries 500)
                         (or config.max_bytes 65536)))
          (each [name value (pairs {:max_bytes max-bytes
                                    :max_entries max-entries})]
            (assert (and (and (= (type value) :number) (>= value 1))
                         (= (% value 1) 0))
                    (.. :history. name " must be a positive integer")))

          (fn bounded [entries]
            (var (first bytes) (values (+ (length entries) 1) 0))
            (for [index (length entries) 1 (- 1)]
              (when (or (>= (- (length entries) index) max-entries)
                        (> (+ bytes (length (. entries index))) max-bytes))
                (lua :break))
              (set bytes (+ bytes (length (. entries index))))
              (set first index))
            (local result {})
            (for [index first (length entries)]
              (tset result (+ (length result) 1) (. entries index)))
            result)

          (fn save [history]
            (if (= config.persist false) {}
                [{:data {:entries history.entries :version 1}
                  :namespace :history
                  :type :state/save}]))

          (each [name keys (pairs {:next [:ctrl_n :alt+n]
                                   :previous [:ctrl_p :alt+p]
                                   :search [:ctrl_r]})]
            (misa.reg_keybinding {:action name :context :history :default keys})
            (misa.reg_action {: available
                              :binding {:action name :context :history}
                              :event {:type (.. :history/ name)}
                              :id (.. :history. name)
                              :label (or (and (= name :search)
                                              "Search input history")
                                         (or (and (= name :previous)
                                                  "Previous input")
                                             "Next input"))}))
          (misa.reg_event :app/start
                          (fn [db]
                            (state db)
                            {: db
                             :fx (or (and (= config.persist false) {})
                                     [{:completion :history/loaded
                                       :namespace :history
                                       :type :state/load}])}))
          (misa.reg_event :history/loaded
                          (fn [db event]
                            (if (not= event.namespace :history) nil
                                (do
                                  (local history (state db))
                                  (local entries {})
                                  (when (not= event.found false)
                                    (local data event.data)
                                    (assert (and (and (= (type data) :table)
                                                      (= data.version 1))
                                                 (= (type data.entries) :table))
                                            "invalid input history data")
                                    (each [_ text (ipairs data.entries)]
                                      (assert (= (type text) :string)
                                              "history entry must be a string")
                                      (when (<= (length text) max-bytes)
                                        (add entries text))))
                                  ;; Input may have been accepted while the native load was outstanding.
                                  (each [_ text (ipairs history.entries)]
                                    (add entries text))
                                  (set history.entries (bounded entries))
                                  {: db}))))
          (misa.reg_event :agent/submitted
                          (fn [db event]
                            (local history (state db))
                            (set (history.index history.draft history.current)
                                 (values 0 nil nil))
                            (if (or (or (not= (type event.prompt) :string)
                                        (> (length event.prompt) max-bytes))
                                    (not (add history.entries event.prompt)))
                                {: db}
                                (do
                                  (set history.entries
                                       (bounded history.entries))
                                  {: db :fx (save history)}))))

          (fn navigate [db direction]
            (if (not (available db)) {: db :fx [{:type :terminal/read}]}
                (do
                  (local (history editor) (values (state db) db.editor))
                  (if (= (length history.entries) 0)
                      {: db :fx [{:type :terminal/read}]}
                      (do
                        (when (or (= history.index 0)
                                  (not= editor.text history.current))
                          (set history.index 0)
                          (set history.draft
                               {:attachments (misa.snapshot (or editor.attachments
                                                                {}))
                                :cursor editor.cursor
                                :text (or editor.text "")}))
                        (local index
                               (math.max 0
                                         (math.min (length history.entries)
                                                   (+ history.index direction))))
                        (if (= index history.index)
                            {: db :fx [{:type :terminal/read}]}
                            (do
                              (set history.index index)
                              (if (= index 0)
                                  (do
                                    (local draft history.draft)
                                    (set history.current nil)
                                    {: db
                                     :fx [(restore draft.text draft.attachments
                                                   draft.cursor)
                                          {:type :terminal/read}]})
                                  (do
                                    (local text
                                           (. history.entries
                                              (+ (- (length history.entries)
                                                    index)
                                                 1)))
                                    (set history.current text)
                                    {: db
                                     :fx [(restore text)
                                          {:type :terminal/read}]})))))))))

          (misa.reg_event :history/previous (fn [db] (navigate db 1)))
          (misa.reg_event :history/next (fn [db] (navigate db (- 1))))
          (misa.reg_event :history/search
                          (fn [db]
                            (if (not (available db))
                                {: db :fx [{:type :terminal/read}]}
                                (do
                                  (local history (state db))
                                  (local (items seen) (values {} {}))
                                  (for [index (length history.entries) 1 (- 1)]
                                    (local text (. history.entries index))
                                    (when (not (. seen text))
                                      (tset seen text true)
                                      (local first
                                             (: (: (or (text:match "[^\n]+")
                                                       "(blank input)")
                                                   :gsub "\t" " ")
                                                :gsub "[%z\001-\031\127]" ""))
                                      (var label
                                           (or (and misa.layout
                                                    (misa.layout.clip first 100))
                                               first))
                                      (when (< (length label) (length first))
                                        (set label (.. label "…")))
                                      (local (_ newlines) (text:gsub "\n" ""))
                                      (tset items (+ (length items) 1)
                                            {:description (or (and (> newlines
                                                                      0)
                                                                   (.. (+ newlines
                                                                          1)
                                                                       " lines"))
                                                              nil)
                                             :id (tostring index)
                                             : label
                                             :search text
                                             :value text})))
                                  (set history.sequence (+ history.sequence 1))
                                  {: db
                                   :fx [{:event {:completion :history/selected
                                                 :id :input-history
                                                 :session (misa.choice_session {: items
                                                                                :purpose :history
                                                                                :title "Input history"
                                                                                :views [:all]}
                                                                               db)
                                                 :title "Input history"
                                                 :token (tostring history.sequence)
                                                 :type :picker/open}
                                         :type :dispatch}]}))))
          (misa.reg_event :history/selected
                          (fn [db event]
                            (local history (state db))
                            (if (or (not= event.picker :input-history)
                                    (not= event.picker_token
                                          (tostring history.sequence)))
                                nil
                                (if event.cancelled
                                    {: db :fx [{:type :terminal/read}]}
                                    (do
                                      (assert (= (type event.value) :string)
                                              "history selection must be text")
                                      (set (history.index history.draft
                                                          history.current)
                                           (values 0 nil nil))
                                      {: db
                                       :fx [(restore event.value)
                                            {:type :terminal/read}]})))))
          (misa.reg_interceptor {:before (fn [tx]
                                           (if (or (not= tx.event.type
                                                         :terminal/input)
                                                   (not (available tx.db)))
                                               tx
                                               (do
                                                 (var action
                                                      (misa.keybinding_action :history
                                                                              tx.event))
                                                 (local editor tx.db.editor)
                                                 (local (text cursor)
                                                        (values (or editor.text
                                                                    "")
                                                                (or editor.cursor
                                                                    0)))
                                                 (when (and (and (and (not action)
                                                                      (= (or editor.mode
                                                                             :insert)
                                                                         :insert))
                                                                 (= tx.event.kind
                                                                    :arrow_up))
                                                            (not (: (text:sub 1
                                                                              cursor)
                                                                    :find "\n" 1
                                                                    true)))
                                                   (set action :previous))
                                                 (when (and (and (and (not action)
                                                                      (= (or editor.mode
                                                                             :insert)
                                                                         :insert))
                                                                 (= tx.event.kind
                                                                    :arrow_down))
                                                            (not (: (text:sub (+ cursor
                                                                                 1))
                                                                    :find "\n" 1
                                                                    true)))
                                                   (set action :next))
                                                 (when action
                                                   (set tx.event
                                                        {:type (.. :history/
                                                                   action)}))
                                                 tx)))
                                 :id :history/input})
          nil)}

