;; Command palette composed from registered commands and canonical recent

;; invocations. Argument completion is an ordinary narrowing source.

(fn command-items [db]
  (let [(result seen) (values {} {})]
    (each [_ command (ipairs (misa.commands))]
      (var narrow nil)
      (when (or command.completion command.complete)
        (set narrow {:context {:command command.name}
                     :source :command-arguments}))
      (tset result (+ (length result) 1)
            {:display {:description command.description :label command.name}
             :id command.name
             :invocation (or (and (not narrow) command.name) nil)
             : narrow
             :path command.name
             :preview {:description command.description :title command.name}
             :search [command.name command.description]
             :value command.name})
      (tset seen command.name true))
    (each [_ entry (ipairs (misa.command_recent db))]
      (when (not (. seen entry.canonical))
        (tset seen entry.canonical true)
        (tset result (+ (length result) 1)
              {:display {:description "recent invocation"
                         :label entry.canonical}
               :id entry.canonical
               :invocation entry.canonical
               :path entry.canonical
               :preview {:kind :recent :title entry.canonical}
               :search [entry.command entry.arguments]
               :value entry.canonical})))
    result))

{:setup (fn []
          (assert (and misa.reg_choice_source misa.command_invocation)
                  "omnipicker requires choices and commands")
          (misa.reg_choice_source :omnipicker
                                  {:items (fn [_ db]
                                            {:input_prefix "/"
                                             :items (command-items db)
                                             :preference_scope :commands
                                             :purpose :command-completion
                                             :title :Commands})})
          (misa.reg_choice_source :command-arguments
                                  {:items (fn [context db]
                                            (misa.command_choice_spec (assert (misa.command context.command))
                                                                      "" db))})

          (fn misa.omnipicker_session [db query]
            (local spec (misa.choice_source :omnipicker nil db))
            (set spec.query (or query ""))
            (misa.choice_session spec db))

          (misa.reg_keybinding {:action :open_omnipicker
                                :context :global
                                :default [:alt+/]})
          (misa.reg_action {:binding {:action :open_omnipicker
                                      :context :global}
                            :event {:type :omnipicker/open}
                            :id :commands.open
                            :label "Open session commands"})
          (misa.reg_event :omnipicker/open
                          (fn [db]
                            (set db.omnipicker_sequence
                                 (+ (or db.omnipicker_sequence 0) 1))
                            (local token
                                   (.. "omnipicker:" db.omnipicker_sequence))
                            {: db
                             :fx [{:event {:completion :omnipicker/selected
                                           :id :omnipicker
                                           :nested (not= db.picker nil)
                                           :session (misa.omnipicker_session db
                                                                             "")
                                           :title :Commands
                                           : token
                                           :type :picker/open}
                                   :type :dispatch}]}))
          (misa.reg_event :omnipicker/selected
                          (fn [db event]
                            (if (not= event.picker :omnipicker) nil
                                (if event.cancelled
                                    {: db :fx [{:type :terminal/read}]}
                                    (do
                                      (local invocation
                                             (assert (misa.command_invocation (or event.invocation
                                                                                  (tostring event.value)))
                                                     "invalid command palette invocation"))
                                      {: db
                                       :fx [{:event invocation :type :dispatch}]})))))
          (misa.reg_interceptor {:before (fn [tx]
                                           (when (and (and (= tx.event.type
                                                              :terminal/input)
                                                           (not tx.db.picker))
                                                      (= (misa.keybinding_action :global
                                                                                 tx.event)
                                                         :open_omnipicker))
                                             (set tx.event
                                                  {:type :omnipicker/open}))
                                           tx)
                                 :id :omnipicker/global})
          nil)}

