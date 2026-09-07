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
          (local setup-fx [])
          (assert (and (misa.has_setup_effect :register/choice-source)
                       misa.command_invocation)
                  "omnipicker requires choices and commands")
          (table.insert setup-fx
                        {:type :register/choice-source
                         :id :omnipicker
                         :value {:items (fn [_ db]
                                          {:input_prefix "/"
                                           :items (command-items db)
                                           :preference_scope :commands
                                           :purpose :command-completion
                                           :title :Commands})}})
          (table.insert setup-fx
                        {:type :register/choice-source
                         :id :command-arguments
                         :value {:items (fn [context db]
                                          (misa.command_choice_spec (assert (misa.command context.command))
                                                                    "" db))}})
          (table.insert setup-fx
                        {:type :register/service
                         :name :omnipicker_session
                         :value (fn [db query]
                                  (local spec
                                         (misa.choice_source :omnipicker nil db))
                                  (set spec.query (or query ""))
                                  (misa.choice_session spec db))})
          (table.insert setup-fx
                        {:type :register/keybinding
                         :value {:action :open_omnipicker
                                 :context :global
                                 :default [:alt+/]}})
          (table.insert setup-fx
                        {:type :register/action
                         :value {:binding {:action :open_omnipicker
                                           :context :global}
                                 :event {:type :omnipicker/open}
                                 :id :commands.open
                                 :label "Open session commands"}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :omnipicker/open
                         :handler (fn [db]
                                    (local sequence
                                         (+ (or db.omnipicker_sequence 0) 1))
                                    (local token
                                           (.. "omnipicker:"
                                               sequence))
                                    {:patch {:omnipicker_sequence sequence}
                                     :fx [{:event {:completion :omnipicker/selected
                                                   :id :omnipicker
                                                   :nested (not= db.picker nil)
                                                   :session (misa.omnipicker_session db
                                                                                     "")
                                                   :title :Commands
                                                   : token
                                                   :type :picker/open}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :omnipicker/selected
                         :handler (fn [db event]
                                    (if (not= event.picker :omnipicker) nil
                                        (if event.cancelled
                                            {:fx [{:type :terminal/read}]}
                                            (do
                                              (local invocation
                                                     (assert (misa.command_invocation (or event.invocation
                                                                                          (tostring event.value)))
                                                             "invalid command palette invocation"))
                                              {:fx [{:event invocation
                                                     :type :dispatch}]}))))})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (if (and (and (= tx.event.type
                                                              :terminal/input)
                                                           (not tx.db.picker))
                                                      (= (misa.keybinding_action :global
                                                                                 tx.event)
                                                         :open_omnipicker))
                                             (misa.patch tx {:event (misa.replace {:type :omnipicker/open})})
                                             tx))
                                 :id :omnipicker/global}})
          {:fx setup-fx})}
