(local clipboard (require :misa.clipboard))
{:events {:clipboard/clipboard/copy {:event :clipboard/copy
                                     :priority 14000
                                     :handler (fn [db event cofx]
                                                (clipboard.copy (or cofx.config.clipboard
                                                                    {})
                                                                db event))}
          :clipboard/clipboard/completed {:event :clipboard/completed
                                          :priority 14000
                                          :handler clipboard.completed}}}
