(local definitions (require :tests.declarations))

;; Append a labelled conversation, then read it back through list and load.
(fn []
  (local declarations [])
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn []
                                    {:fx [{:type :conversation/append
                                           :conversation :reading
                                           :entries [{:kind :message
                                                      :data {:content [{:text :one
                                                                        :type :text}]
                                                             :role :user}}
                                                     {:kind :message
                                                      :data {:content [{:text :two
                                                                        :type :text}]
                                                             :role :assistant}}]
                                           :metadata {:label :Reading}
                                           :completion :conversation/appended
                                           :id :append-1}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :conversation/appended
                         :handler (fn [_ event]
                                    (assert (= event.ok true))
                                    {:fx [{:type :conversation/list
                                           :limit 5
                                           :completion :conversations/listed
                                           :id :list-1}]})}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :conversations/listed
                         :handler (fn [_ event]
                                    (assert (= event.ok true))
                                    (let [found (icollect [_ summary (ipairs event.data.conversations)]
                                                  (when (= summary.id :reading)
                                                    summary))]
                                      (assert (= (length found) 1)
                                              "the appended conversation was not listed")
                                      (assert (= (. found 1 :entry_count) 2))
                                      (assert (= (. found 1 :metadata :label)
                                                 :Reading)
                                              "the conversation label was lost")
                                      {:fx [{:type :conversation/load
                                             :conversation :reading
                                             :limit 10
                                             :completion :conversation/loaded
                                             :id :load-1}]}))}})
  (table.insert declarations
                {:catalog :events
                 :value {:event :conversation/loaded
                         :handler (fn [_ event]
                                    (assert (= event.ok true))
                                    (assert (= event.data.conversation :reading))
                                    (assert (= (length event.data.entries) 2))
                                    (assert (= event.data.more_entries false))
                                    (assert (= (. event.data.entries 1 :seq) 1))
                                    (assert (= (. event.data.entries 1 :data
                                                  :role)
                                               :user))
                                    (assert (= (. event.data.entries 1 :data
                                                  :content 1 :text)
                                               :one))
                                    (assert (= (. event.data.entries 2 :data
                                                  :role)
                                               :assistant))
                                    {:fx [{:lines [{:spans [{:text "read 2 entries"}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.conversation-read
    declarations
    {}))
