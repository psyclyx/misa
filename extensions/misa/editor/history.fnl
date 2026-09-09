;; Submitted input history. Navigation and search are policies over ordinary
;; editor restore events and generic choice sessions; drafts are never stored.

(fn available? [db]
  "Return whether history navigation is available."
  (and (not= db.editor nil) (not db.picker) (not db.dialog) (not db.selection)
       (not db.editor.choice)))

(fn state [db]
  (or db.history {:entries {} :index 0 :sequence 0}))

(fn updated [history patch fx]
  {:patch {:history (misa.replace (misa.patch history patch))} : fx})

(fn reset-navigation []
  {:index 0 :draft misa.delete :current misa.delete})

(fn restore [text attachments cursor]
  {:event {:attachments (or attachments {})
           : cursor
           :replace true
           : text
           :type :editor/restore}
   :type :dispatch})

(fn add [entries text]
  (if (and (= (type text) :string) (not= text "")
           (not= (. entries (length entries)) text))
      (do
        (tset entries (+ (length entries) 1) text)
        true)
      false))

(fn navigate [db direction]
  "Move through submitted inputs while preserving the editor draft."
  (if (not (available? db)) {:fx [{:type :terminal/read}]}
      (let [history (state db)
            editor db.editor]
        (if (= (length history.entries) 0)
            {:fx [{:type :terminal/read}]}
            (let [reset (or (= history.index 0)
                            (not= editor.text history.current))
                  start (if reset 0 history.index)
                  draft (if reset
                            {:attachments (or editor.attachments {})
                             :cursor editor.cursor
                             :text (or editor.text "")}
                            history.draft)
                  index (math.max 0
                                  (math.min (length history.entries)
                                            (+ start direction)))
                  text (when (> index 0)
                         (. history.entries
                            (+ (- (length history.entries) index) 1)))
                  fx [{:type :terminal/read}]]
              (when (not= index start)
                (table.insert fx 1
                              (if (= index 0)
                                  (restore draft.text draft.attachments
                                           draft.cursor)
                                  (restore text))))
              (updated history
                       {: index
                        :draft (misa.replace draft)
                        :current (misa.replace (if (= index start)
                                                   history.current
                                                   text))}
                       fx))))))

(fn on-history-search [db]
  "Open a picker over saved input history."
  (if (not (available? db))
      {:fx [{:type :terminal/read}]}
      (let [history (state db)
            items {}
            seen {}]
        (for [index (length history.entries) 1 (- 1)]
          (let [text (. history.entries index)]
            (when (not (. seen text))
              (tset seen text true)
              (let [first (: (: (or (text:match "[^
]+") "(blank input)") :gsub "\t" " ") :gsub
                             "[%z\001-\031\127]" "")]
                (var label (or (and misa.layout (misa.layout.clip first 100))
                               first))
                (when (< (length label) (length first))
                  (set label (.. label "…")))
                (let [(_ newlines) (text:gsub "\n" "")]
                  (tset items (+ (length items) 1)
                        {:description (or (and (> newlines 0)
                                               (.. (+ newlines 1) " lines"))
                                          nil)
                         :id (tostring index)
                         : label
                         :search text
                         :value text}))))))
        (let [sequence (+ history.sequence 1)]
          {:patch {:history (misa.replace (misa.patch history {: sequence}))}
           :fx [{:event {:completion :history/selected
                         :id :input-history
                         :session (misa.choices.session {: items
                                                         :purpose :history
                                                         :title "Input history"
                                                         :views [:all]}
                                                        db)
                         :title "Input history"
                         :token (tostring sequence)
                         :type :picker/open}
                 :type :dispatch}]}))))

(fn on-history-selected [db event]
  "Restore the selected history entry to the draft."
  (let [history (state db)]
    (if (or (not= event.picker :input-history)
            (not= event.picker_token (tostring history.sequence)))
        nil
        (if event.cancelled
            {:fx [{:type :terminal/read}]}
            (do
              (assert (= (type event.value) :string)
                      "history selection must be text")
              (updated history (reset-navigation)
                       [(restore event.value) {:type :terminal/read}]))))))

(fn route-terminal-input [db event]
  "Route terminal input to history navigation."
  (when (available? db)
    (var action (misa.keybindings.action :history event))
    (let [editor db.editor
          text (or editor.text "")
          cursor (or editor.cursor 0)]
      (when (and (not action) (= (or editor.mode :insert) :insert))
        (if (and (= event.kind :arrow_up)
                 (not (: (text:sub 1 cursor) :find "\n" 1 true)))
            (set action :previous)
            (and (= event.kind :arrow_down)
                 (not (: (text:sub (+ cursor 1)) :find "\n" 1 true)))
            (set action :next)))
      (when action
        {:type (.. :history/ action)}))))

(fn recent-entries [entries max-entries max-bytes]
  "Return the newest contiguous history that fits the entry and byte limits."
  (var first (+ (length entries) 1))
  (var bytes 0)
  (while (and (> first 1) (< (+ (- (length entries) first) 1) max-entries)
              (<= (+ bytes (length (. entries (- first 1)))) max-bytes))
    (set first (- first 1))
    (set bytes (+ bytes (length (. entries first)))))
  (fcollect [index first (length entries)] (. entries index)))

(fn options [config]
  "Validate history persistence and retention settings."
  (let [value (or config.history {})
        settings (if (= (type value) :table) value {})
        result {:persist settings.persist
                :max_entries (or settings.max_entries 500)
                :max_bytes (or settings.max_bytes 65536)}]
    (each [_ limit (ipairs [result.max_entries result.max_bytes])]
      (assert (and (= (type limit) :number) (>= limit 1) (= (% limit 1) 0))
              "history limits must be positive integers"))
    result))

(fn save [config history]
  "Save."
  (if (= config.persist false) {}
      [{:data {:entries history.entries :version 1}
        :namespace :history
        :type :state/save}]))

(fn on-app-start [config db]
  "Initialize input history and request persisted entries."
  (let [config (options config)
        max-entries config.max_entries
        max-bytes config.max_bytes]
    {:patch {:history (misa.replace (state db))}
     :fx (or (and (= config.persist false) {})
             [{:completion :history/loaded
               :namespace :history
               :type :state/load}])}))

(fn on-history-loaded [config db event]
  "Merge saved history with input accepted while loading."
  (let [config (options config)
        max-entries config.max_entries
        max-bytes config.max_bytes]
    (if (not= event.namespace :history) nil
        (let [history (state db)
              entries {}]
          (when (not= event.found false)
            (let [data event.data]
              (assert (and (= (type data) :table) (= data.version 1)
                           (= (type data.entries) :table))
                      "invalid input history data")
              (each [_ text (ipairs data.entries)]
                (assert (= (type text) :string)
                        "history entry must be a string")
                (when (<= (length text) max-bytes)
                  (add entries text)))))
          ;; Input may have been accepted while the native load was outstanding.
          (each [_ text (ipairs history.entries)]
            (add entries text))
          (updated history
                   {:entries (misa.replace (recent-entries entries max-entries
                                                           max-bytes))})))))

(fn on-agent-submitted [config db event]
  "Record accepted input and trim history to configured limits."
  (let [config (options config)
        max-entries config.max_entries
        max-bytes config.max_bytes]
    (let [history (state db)
          entries {}]
      (each [index text (ipairs history.entries)]
        (tset entries index text))
      (let [changed (and (= (type event.prompt) :string)
                         (<= (length event.prompt) max-bytes)
                         (add entries event.prompt))
            next-history (misa.patch history (reset-navigation))]
        (if changed
            (let [next-history (misa.patch next-history
                                           {:entries (misa.replace (recent-entries entries
                                                                                   max-entries
                                                                                   max-bytes))})]
              (updated next-history {} (save config next-history)))
            (updated next-history {}))))))

{:available? available?
 :navigate navigate
 :on-agent-submitted on-agent-submitted
 :on-app-start on-app-start
 :on-history-loaded on-history-loaded
 :on-history-search on-history-search
 :on-history-selected on-history-selected
 :recent-entries recent-entries
 :route-terminal-input route-terminal-input}
