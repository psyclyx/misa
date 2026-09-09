(local implementation (require :misa.commands))

{:services {:commands.invocation implementation.invocation
            :commands.canonical implementation.canonical
            :commands.recent implementation.recent
            :commands.choice-items implementation.choice-items
            :commands.choice-spec implementation.choice-spec}
 :events {:commands/commands/invoke {:event :commands/invoke
                                     :handler implementation.invoke
                                     :priority 20000}
          :commands/choices/command-open {:event :choices/command-open
                                          :handler implementation.open-choice
                                          :priority 20000}
          :commands/choices/command-selected {:event :choices/command-selected
                                              :handler implementation.select-choice
                                              :priority 20000}}}
