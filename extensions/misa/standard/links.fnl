(local links (require :misa.links))
{:events {:links/link/open {:event :link/open
                            :priority 17000
                            :handler (fn [db event cofx]
                                       (links.open (links.opener (or cofx.config.links
                                                                     {}))
                                                   db event))}
          :links/link/completed {:event :link/completed
                                 :priority 17000
                                 :handler links.completed}}}
