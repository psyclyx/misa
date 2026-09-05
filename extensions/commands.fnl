;; Canonical command invocation and generic recent replay. Command execution has

;; one normalization path whether input was typed, picked, or replayed.

(fn trim [value]
  (: (tostring (or value "")) :match "^%s*(.-)%s*$"))

(fn canonical [name arguments]
  (let [args (trim arguments)]
    (.. name (or (and (not= args "") (.. " " args)) ""))))

{:setup (fn []
          (fn misa.command_invocation [text]
            (assert (= (type text) :string)
                    "command invocation must be a string")
            (var (name args) (text:match "^(%S+)%s*(.-)%s*$"))
            (local command (and name (misa.command name)))
            (if (not command) nil
                (do
                  (set args (trim args))
                  {:arguments args
                   :canonical (canonical command.name args)
                   :command command.name
                   :normalized_command true
                   :type command.event})))

          (set misa.command_canonical canonical)

          (fn misa.command_recent [db]
            (local result {})
            (local ___values___ (or (and db.preferences
                                         db.preferences.scopes.commands)
                                    {}))
            (each [text usage (pairs ___values___)]
              (local invocation (misa.command_invocation text))
              (when (and invocation (or (> (or usage.uses 0) 0) usage.favorite))
                (tset result (+ (length result) 1)
                      {:arguments invocation.arguments
                       :canonical invocation.canonical
                       :command invocation.command
                       :last (or usage.last 0)})))
            (table.sort result
                        (fn [a b]
                          (if (not= a.last b.last) (> a.last b.last)
                              (< a.canonical b.canonical))))
            result)

          (fn misa.command_choice_items [command query db]
            (local result {})
            (each [_ candidate (ipairs (misa.command_completions command query
                                                                 db))]
              (local item {})
              (each [key value (pairs candidate)] (tset item key value))
              (set item.invocation (canonical command.name item.value))
              (tset result (+ (length result) 1) item))
            result)

          ;; One argument source serves typed completion, narrowing, and explicit
          ;; pickers. Presentation never needs to reconstruct command metadata.

          (fn misa.command_choice_spec [command query db]
            {:input_prefix (.. command.name " ")
             :items (misa.command_choice_items command (or query "") db)
             :preference_scope (or command.preference_scope
                                   (.. "command:" command.name))
             :purpose (or command.choice_purpose :command)
             :query (or query "")
             :selected (and command.selected (command.selected db))
             :title (command.name:sub 2)})

          (misa.reg_interceptor {:before (fn [tx]
                                           (local event tx.event)
                                           (if (not= (type event.command)
                                                     :string)
                                               tx
                                               (do
                                                 (local command
                                                        (misa.command event.command))
                                                 (if (not command) tx
                                                     (do
                                                       (local args
                                                              (trim event.arguments))
                                                       (if (and (and (= args "")
                                                                     (or command.completion
                                                                         command.complete))
                                                                (not event.resumed_choice))
                                                           (do
                                                             (set tx.event
                                                                  {:command command.name
                                                                   :type :choices/command-open})
                                                             tx)
                                                           (do
                                                             (set event.arguments
                                                                  args)
                                                             (set event.canonical
                                                                  (canonical command.name
                                                                             args))
                                                             (set event.normalized_command
                                                                  true)
                                                             ;; Invocation normalization owns usage, so typing, inline choices, and
                                                             ;; overlay replay all update the same canonical preference exactly once.
                                                             (when misa.preference_use
                                                               (var save
                                                                    (misa.preference_use tx.db
                                                                                         :commands
                                                                                         event.canonical))
                                                               (when (and (not= args
                                                                                "")
                                                                          (or command.completion
                                                                              command.complete))
                                                                 (each [_ candidate (ipairs (misa.command_completions command
                                                                                                                      args
                                                                                                                      tx.db))]
                                                                   (when (= candidate.value
                                                                            args)
                                                                     (set save
                                                                          (misa.preference_use tx.db
                                                                                               (or command.preference_scope
                                                                                                   (.. "command:"
                                                                                                       command.name))
                                                                                               (or candidate.id
                                                                                                   (tostring candidate.value))))
                                                                     (lua :break))))
                                                               (tset tx.fx
                                                                     (+ (length tx.fx)
                                                                        1)
                                                                     save))
                                                             tx)))))))
                                 :id :commands/normalize})
          (misa.reg_event :choices/command-open
                          (fn [db event]
                            (local command
                                   (assert (misa.command event.command)))
                            (set db.choice_commands
                                 (or db.choice_commands
                                     {:pending {} :sequence 0}))
                            (local state db.choice_commands)
                            (set state.sequence (+ state.sequence 1))
                            (local token (.. "command:" state.sequence))
                            (tset state.pending token {:command command.name})
                            {: db
                             :fx [{:event {:completion :choices/command-selected
                                           :id :command-choice
                                           :session (misa.choice_session (misa.command_choice_spec command
                                                                                                   ""
                                                                                                   db)
                                                                         db)
                                           :title (command.name:sub 2)
                                           : token
                                           :type :picker/open}
                                   :type :dispatch}]}))
          (misa.reg_event :choices/command-selected
                          (fn [db event]
                            (local pending
                                   (and db.choice_commands
                                        (. db.choice_commands.pending
                                           event.picker_token)))
                            (if (or (not= event.picker :command-choice)
                                    (not pending))
                                nil
                                (do
                                  (tset db.choice_commands.pending
                                        event.picker_token nil)
                                  (if event.cancelled
                                      {: db :fx [{:type :terminal/read}]}
                                      (do
                                        (local invocation
                                               (assert (misa.command_invocation (.. pending.command
                                                                                    " "
                                                                                    (tostring event.value)))))
                                        (set invocation.resumed_choice true)
                                        {: db
                                         :fx [{:event invocation
                                               :type :dispatch}]}))))))
          nil)}

