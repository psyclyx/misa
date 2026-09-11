(local {: available-images-remove
        : compute-images-lifecycle
        : on-agent-reset
        : on-images-loaded
        : request} (require :misa.editor.images))

{:actions {:images.paste {:available (fn [db]
                                       (and (not db.dialog) (not db.picker)))
                          :event {:type :images/paste}
                          :id :images.paste
                          :keys [:ctrl_v]
                          :label "Paste image from clipboard"}
           :images.remove {:available available-images-remove
                           :event {:type :editor/detach}
                           :id :images.remove
                           :label "Remove last draft attachment"}}
 :commands {:/image {:description "Attach a PNG or JPEG file"
                     :event :images/load
                     :name :/image}}
 :events {:images/images/paste {:event :images/paste
                                :handler (fn [db event cofx]
                                           (request cofx.config db event true))
                                :priority 68000}
          :images/images/load {:event :images/load
                               :handler (fn [db event cofx]
                                          (request cofx.config db event false))
                               :priority 68000}
          :images/images/loaded {:event :images/loaded
                                 :handler on-images-loaded
                                 :priority 68000}
          :images/agent/reset {:event :agent/reset
                               :handler on-agent-reset
                               :priority 68000}}
 :services {:editor.lifecycle.images [:images/lifecycle]}
 :subscriptions {:images/lifecycle {:id :images/lifecycle
                                    :inputs [[:db/path :images :pending]]
                                    :compute compute-images-lifecycle}}}
