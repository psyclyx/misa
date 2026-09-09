(local definitions (require :misa.definitions))

;; Canonical command invocation and generic recent replay. Command execution has
;; one normalization path whether input was typed, picked, or replayed.

(fn trim [value]
  (: (tostring (or value "")) :match "^%s*(.-)%s*$"))

(fn canonical [name arguments]
  "Join a command name and trimmed arguments into canonical input."
  (let [args (trim arguments)]
    (.. name (or (and (not= args "") (.. " " args)) ""))))

(fn invoke [db event]
  (let [command (assert (misa.commands.lookup event.command) "unknown command")
        args (trim event.arguments)]
    (if (and (= args "") (or command.completion command.complete)
             (not event.resumed_choice))
        {:fx [{:type :dispatch
               :event {:type :choices/command-open :command command.name}}]}
        (let [invocation (canonical command.name args)
              execution (misa.patch event
                                    {:type command.event
                                     :arguments args
                                     :canonical invocation})
              effects []]
          (var patch {})
          (when (and misa.preferences misa.preferences.use)
            (var preferences (misa.preferences.use db :commands invocation))
            (let [used (misa.patch db {:preferences (misa.replace preferences)})]
              (when (and (not= args "")
                         (or command.completion command.complete))
                (each [_ candidate (ipairs (misa.commands.completions command
                                                                      args used))]
                  (when (= candidate.value args)
                    (set preferences
                         (misa.preferences.use used
                                               (or command.preference_scope
                                                   (.. "command:" command.name))
                                               (or candidate.id
                                                   (tostring candidate.value))))
                    (lua :break))))
              (set patch {:preferences (misa.replace preferences)})
              (table.insert effects
                            {:type :state/save
                             :namespace :preferences
                             :data preferences})))
          (table.insert effects {:type :dispatch :event execution})
          {: patch :fx effects}))))

(fn build []
  "Build the declarations for commands."
  (definitions.build :commands
    [{:catalog :services
      :id :commands.invocation
      :value (fn [text]
               "Parse input into a registered command invocation, or return nil."
               (assert (= (type text) :string)
                       "command invocation must be a string")
               (var (name args) (text:match "^(%S+)%s*(.-)%s*$"))
               (let [command (and name (misa.commands.lookup name))]
                 (if (not command) nil
                     (do
                       (set args (trim args))
                       {:arguments args
                        :canonical (canonical command.name args)
                        :command command.name
                        :type :commands/invoke}))))}
     {:catalog :services :id :commands.canonical :value canonical}
     {:catalog :services
      :id :commands.recent
      :value (fn [db]
               "List previously used commands, most recent first."
               (let [result {}
                     source-values (or (and db.preferences
                                            db.preferences.scopes.commands)
                                       {})]
                 (each [text usage (pairs source-values)]
                   (let [invocation (misa.commands.invocation text)]
                     (when (and invocation
                                (or (> (or usage.uses 0) 0) usage.favorite))
                       (tset result (+ (length result) 1)
                             {:arguments invocation.arguments
                              :canonical invocation.canonical
                              :command invocation.command
                              :last (or usage.last 0)}))))
                 (table.sort result
                             (fn [a b]
                               (if (not= a.last b.last)
                                   (> a.last b.last)
                                   (< a.canonical b.canonical))))
                 result))}
     {:catalog :services
      :id :commands.choice-items
      :value (fn [command query db]
               "Return completion items with canonical command invocations."
               (let [result {}]
                 (each [_ candidate (ipairs (misa.commands.completions command
                                                                       query db))]
                   (let [item {}]
                     (each [key value (pairs candidate)]
                       (tset item key value))
                     (set item.invocation (canonical command.name item.value))
                     (tset result (+ (length result) 1) item)))
                 result))}
     {:catalog :services
      :id :commands.choice-spec
      :value (fn [command query db]
               "Build a choice specification for a command and query."
               {:input_prefix (.. command.name " ")
                :items (misa.commands.choice-items command (or query "") db)
                :preference_scope (or command.preference_scope
                                      (.. "command:" command.name))
                :purpose (or command.choice_purpose :command)
                :query (or query "")
                :selected (and command.selected (command.selected db))
                :title (command.name:sub 2)})}
     {:catalog :events :value {:event :commands/invoke :handler invoke}}
     {:catalog :events
      :value {:event :choices/command-open
              :handler (fn [db event]
                         (let [command (assert (misa.commands.lookup event.command))]
                           (if (and command.choice_available
                                    (not (command.choice_available db)))
                               {:fx [{:type :dispatch
                                      :event {:type command.choice_unavailable}}]}
                               (do
                                 (let [state (or db.choice_commands
                                                 {:pending {} :sequence 0})
                                       sequence (+ state.sequence 1)
                                       token (.. "command:" sequence)]
                                   {:patch {:choice_commands {: sequence
                                                              :pending {token {:command command.name}}}}
                                    :fx [{:event {:completion :choices/command-selected
                                                  :id :command-choice
                                                  :session (misa.choices.session (misa.commands.choice-spec command
                                                                                                            ""
                                                                                                            db)
                                                                                 db)
                                                  :title (command.name:sub 2)
                                                  : token
                                                  :type :picker/open}
                                          :type :dispatch}]})))))}}
     {:catalog :events
      :value {:event :choices/command-selected
              :handler (fn [db event]
                         (let [pending (and db.choice_commands
                                            (. db.choice_commands.pending
                                               event.picker_token))]
                           (if (or (not= event.picker :command-choice)
                                   (not pending))
                               nil
                               (do
                                 (let [patch {:choice_commands {:pending {event.picker_token misa.delete}}}]
                                   (if event.cancelled
                                       {: patch :fx [{:type :terminal/read}]}
                                       (do
                                         (let [invocation (assert (misa.commands.invocation (.. pending.command
                                                                                                " "
                                                                                                (tostring event.value))))]
                                           (set invocation.resumed_choice true)
                                           {: patch
                                            :fx [{:event invocation
                                                  :type :dispatch}]}))))))))}}]
    {}))

{: build}
