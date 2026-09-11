(local conversation (require :misa.conversation))

{:commands {:/resume {:description "Resume a stored conversation"
                      :event :conversation/open
                      :name :/resume}}
 :events {:conversation/app/start {:event :app/start
                                   :handler conversation.start}
          :conversation/agent/completed {:event :agent/completed
                                         :handler conversation.completed}
          :conversation/conversation/open {:event :conversation/open
                                           :handler conversation.open}
          :conversation/conversations/listed {:event :conversations/listed
                                              :handler conversation.listed}
          :conversation/conversation/loaded {:event :conversation/loaded
                                             :handler conversation.loaded}
          :conversation/conversation/selected {:event :conversation/selected
                                               :handler conversation.selected}
          :conversation/conversation/appended {:event :conversation/appended
                                               :handler conversation.appended}}}
