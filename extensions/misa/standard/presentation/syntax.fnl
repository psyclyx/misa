(local syntax (require :misa.transcript.syntax))
{:subscriptions {:syntax/projections {:id :syntax/projections
                                      :inputs [[:db/path :syntax :documents]]
                                      :compute syntax.syntax-projections-value}
                 :syntax/projection {:id :syntax/projection
                                     :inputs [[:syntax/projections]]
                                     :compute (fn [inputs query]
                                                (let [entry (. (. inputs 1)
                                                               (. query 2))]
                                                  (and entry entry.value)))}}
 :services {:syntax.all (fn [db] (misa.sub db [:syntax/projections]))
            :syntax.for-model syntax.syntax-for-model}
 :events {:syntax/app/start {:event :app/start :handler syntax.initialize}
          :syntax/transcript/reset {:event :transcript/reset
                                    :handler syntax.reset}
          :syntax/transcript/updated {:event :transcript/updated
                                      :handler (fn [db event cofx]
                                                 (syntax.transcript-updated (not= (. (or cofx.config.messages
                                                                                         {})
                                                                                     :markdown)
                                                                                  false)
                                                                            syntax.update-models
                                                                            db
                                                                            event
                                                                            cofx))}
          :syntax/syntax/completed {:event :syntax/completed
                                    :handler (fn [db event]
                                               (syntax.syntax-completed syntax.request
                                                                        db event))}}}
