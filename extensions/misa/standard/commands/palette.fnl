(local implementation (require :misa.commands.palette))

{:choice-sources {:omnipicker {:items (fn [_ db]
                                        {:input_prefix "/"
                                         :items (implementation.command-items db)
                                         :preference_scope :commands
                                         :purpose :command-completion
                                         :title :Commands})}
                  :command-arguments {:items (fn [context db]
                                               (misa.commands.choice-spec (assert (misa.commands.lookup context.command))
                                                                          "" db))}}
 :services {:picker.session implementation.session}
 :keybindings {:global/open_omnipicker {:action :open_omnipicker
                                        :context :global
                                        :default [:alt+/]}}
 :actions {:commands.open {:binding {:action :open_omnipicker :context :global}
                           :event {:type :omnipicker/open}
                           :id :commands.open
                           :label "Open session commands"}}
 :events {:omnipicker/omnipicker/open {:event :omnipicker/open
                                       :handler implementation.open-picker
                                       :priority 60000}
          :omnipicker/omnipicker/selected {:event :omnipicker/selected
                                           :handler implementation.select-command
                                           :priority 60000}}
 :requirements {:omnipicker [:commands.invocation]}}
