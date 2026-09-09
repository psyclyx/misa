(local transcript (require :misa.transcript))
(local model (require :misa.transcript.model))
(local presentation (require :misa.transcript.presentation))
(local viewport (require :misa.transcript.viewport))
(local policies (setmetatable {} {:__mode :k}))

(fn settings [configuration] (or configuration.messages {}))
(fn markdown? [config]
  (and (not= config.plain true) (not= config.markdown false)))

(fn policy [configuration]
  (or (. policies configuration) (let [config (settings configuration)
                                       value {:max_depth (or config.max_depth 8)
                                              :max_items (or config.max_items
                                                             64)
                                              :max_string (or config.max_string
                                                              4000)
                                              :redact (collect [_ key (ipairs (or config.redact_keys
                                                                                  [:authorization
                                                                                   :api_key
                                                                                   :password
                                                                                   :secret
                                                                                   :token]))]
                                                        (: (tostring key)
                                                           :lower)
                                                        true)}]
                                   (assert (and (= (type value.max_string)
                                                   :number)
                                                (> value.max_string 0))
                                           "messages.max_string must be positive")
                                   (tset policies configuration value)
                                   value)))

(local scroll-inputs
       {:wheel_up (fn [] 3)
        :wheel_down (fn [] -3)
        :transcript_up (fn [cofx]
                         (math.max 1 (math.floor (/ cofx.terminal.lines 2))))
        :transcript_down (fn [cofx]
                           (- (math.max 1
                                        (math.floor (/ cofx.terminal.lines 2)))))})

{:transcript-presentations presentation.presentations
 :transcript-deltas {:assistant model.text-delta
                     :thinking model.text-delta
                     :tool_call model.tool-delta}
 :services {:transcript.state presentation.state
            :transcript.viewport viewport.viewport
            :transcript.blocks model.blocks
            :transcript.window transcript.transcript-window}
 :projections {:transcript.project {:inputs transcript.projection-inputs
                                    :render (fn [db context]
                                              (let [config (settings (or (and context
                                                                              context.config)
                                                                         (misa.configuration)))]
                                                (presentation.project (markdown? config)
                                                                      (misa.catalog :transcript-presentations)
                                                                      db context)))}}
 :selection-sources {:transcript {:documents presentation.documents
                                  :layout presentation.document-layout}}
 :keybindings {:global/toggle_verbose {:action :toggle_verbose
                                       :context :global
                                       :default [:alt+t]}
               :global/transcript_up {:action :transcript_up
                                      :context :global
                                      :default [:page_up :alt+k]}
               :global/transcript_down {:action :transcript_down
                                        :context :global
                                        :default [:page_down :alt+j]}}
 :actions {:transcript.detail {:id :transcript.detail
                               :binding {:action :toggle_verbose
                                         :context :global}
                               :event {:type :messages/toggle-verbose}
                               :label "Toggle transcript detail"}
           :transcript.up {:id :transcript.up
                           :binding {:action :transcript_up :context :global}
                           :event {:delta 10 :type :messages/scroll}
                           :label "Scroll transcript up"}
           :transcript.down {:id :transcript.down
                             :binding {:action :transcript_down
                                       :context :global}
                             :event {:delta -10 :type :messages/scroll}
                             :label "Scroll transcript down"}}
 :routes {:messages/global-keys {:id :messages/global-keys
                                 :event :terminal/input
                                 :priority 400
                                 :context [:db/path]
                                 :resolve (fn [db event cofx]
                                            (transcript.terminal-input scroll-inputs
                                                                       db event
                                                                       cofx))}}
 :indicators {:transcript-detail {:id :transcript-detail
                                  :hotkey {:action :toggle_verbose
                                           :context :global}
                                  :icon "≡"
                                  :label :detail
                                  :query [:messages/detail-indicator]}}
 :subscriptions {:messages/detail-indicator {:id :messages/detail-indicator
                                             :inputs [[:db/path
                                                       :messages
                                                       :verbose]]
                                             :compute transcript.messages-detail-indicator-value}}
 :events {:messages/app/start {:event :app/start
                               :handler (fn [db _ cofx]
                                          (policy cofx.config)
                                          (model.initialize (settings cofx.config)
                                                            db))}
          :messages/messages/toggle-verbose {:event :messages/toggle-verbose
                                             :handler model.toggle-detail}
          :messages/messages/scroll {:event :messages/scroll
                                     :handler viewport.handle-scroll}
          :messages/transcript/reset {:event :transcript/reset
                                      :handler model.reset}
          :messages/transcript/response-start {:event :transcript/response-start
                                               :handler model.start-response}
          :messages/transcript/block-start {:event :transcript/block-start
                                            :handler model.start-block}
          :messages/transcript/block-delta {:event :transcript/block-delta
                                            :handler (fn [db event cofx]
                                                       (model.update-block (misa.catalog :transcript-deltas)
                                                                           (policy cofx.config)
                                                                           db
                                                                           event))}
          :messages/transcript/block-end {:event :transcript/block-end
                                          :handler (fn [db event cofx]
                                                     (model.finish-streaming-block (policy cofx.config)
                                                                                   db
                                                                                   event))}
          :messages/transcript/response-end {:event :transcript/response-end
                                             :handler (fn [db event cofx]
                                                        (model.finish-response transcript.noninteractive-commit
                                                                               (markdown? (settings cofx.config))
                                                                               (policy cofx.config)
                                                                               db
                                                                               event
                                                                               cofx))}
          :messages/transcript/response-interrupted {:event :transcript/response-interrupted
                                                     :handler (fn [db
                                                                   event
                                                                   cofx]
                                                                (model.interrupt-response (policy cofx.config)
                                                                                          db
                                                                                          event
                                                                                          cofx))}
          :messages/transcript/user {:event :transcript/user
                                     :handler (fn [db event cofx]
                                                (model.append-message transcript.noninteractive-commit
                                                                      (markdown? (settings cofx.config))
                                                                      :user db
                                                                      event cofx))}
          :messages/transcript/harness {:event :transcript/harness
                                        :handler (fn [db event cofx]
                                                   (model.append-message transcript.noninteractive-commit
                                                                         (markdown? (settings cofx.config))
                                                                         :harness
                                                                         db
                                                                         event
                                                                         cofx))}
          :messages/transcript/tool-start {:event :transcript/tool-start
                                           :handler model.start-tool}
          :messages/transcript/tool-result {:event :transcript/tool-result
                                            :handler model.finish-tool}
          :messages/transcript/tool-summary {:event :transcript/tool-summary
                                             :handler model.summarize-tool}
          :messages/transcript/tool-call {:event :transcript/tool-call
                                          :handler (fn [db event cofx]
                                                     (model.append-tool (policy cofx.config)
                                                                        db event
                                                                        cofx))}
          :messages/transcript/assistant {:event :transcript/assistant
                                          :handler (fn [db event cofx]
                                                     (model.append-assistant transcript.noninteractive-commit
                                                                             (markdown? (settings cofx.config))
                                                                             (policy cofx.config)
                                                                             db
                                                                             event
                                                                             cofx))}
          :messages/transcript/interrupted {:event :transcript/interrupted
                                            :handler model.append-interrupted}
          :messages/runtime/dispatch-limit {:event :runtime/dispatch-limit
                                            :handler transcript.runtime-dispatch-limit}}
 :validators {:transcript-presentations transcript.validate-presentation
              :transcript-deltas transcript.validate-delta}}
