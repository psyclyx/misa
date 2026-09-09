(local {: initialize
        : toggle-detail
        : append-message
        : text-delta
        : tool-delta
        : append-assistant
        : update-block
        : finish-streaming-block
        : start-block
        : blocks
        : append-interrupted
        : reset
        : finish-response
        : interrupt-response
        : start-response
        : append-tool
        : finish-tool
        : start-tool
        : summarize-tool} (require :misa.transcript.model))

(local {: handle-scroll : viewport} (require :misa.transcript.viewport))
(local {: presentations : documents : document-layout : project : state}
       (require :misa.transcript.presentation))

(local definitions (require :misa.definitions))

(fn noninteractive-commit [db role model cofx markdown]
  (if (or cofx.terminal.interactive
          (not (and misa.components misa.components.render)))
      {}
      (let [rendered (misa.components.render db role model
                                             {:columns cofx.terminal.columns
                                              :interactive false
                                              : markdown})]
        (if (= (length (or rendered.lines {})) 0) {}
            [{:lines rendered.lines :type :view/commit}]))))

(fn messages-detail-indicator-value [inputs]
  {:type :text
   :value (if (. inputs 1)
              :verbose
              :summary)})

(fn terminal-input [scroll-inputs db event cofx]
  (when (and (not db.picker) (not db.dialog) misa.keybindings
             misa.keybindings.action)
    (let [action (misa.keybindings.action :global event)
          scroll (or (. scroll-inputs event.kind) (. scroll-inputs action))]
      (if scroll {:type :messages/scroll :delta (scroll cofx)}
          (= action :toggle_verbose) {:type :messages/toggle-verbose}))))

(fn transcript-window [db context room]
  "Return the current visible transcript window."
  (. (viewport db context room) :lines))

(fn runtime-dispatch-limit [_ event]
  {:fx [{:type :dispatch
         :event {:type :transcript/harness :level :error :text event.text}}]})

(fn projection-inputs [db]
  (let [state (assert db.messages "message state is not initialized")]
    {:blocks state.blocks
     :responses state.responses
     :by_response state.by_response
     :verbose state.verbose
     :syntax db.syntax
     :selection db.selection
     :costs db.costs
     :components db.components
     :themes db.themes
     :hover_action db.hover_action
     :hover_link db.hover_link
     :choice_pending (and misa.choices misa.choices.pending
                          (misa.choices.pending db))}))

(fn build [context]
  "Build the declarations for transcript presentation."
  (let [declarations []
        projectors {}]
    (each [id project (pairs presentations)] (tset projectors id project))
    (let [delta-handlers {:assistant text-delta
                          :thinking text-delta
                          :tool_call tool-delta}
          config (let [value (. (or context.config {}) :messages)]
                   (if (= (type value) :table) value {}))
          markdown (and (not= config.plain true) (not= config.markdown false))
          policy {:max_depth (or config.max_depth 8)
                  :max_items (or config.max_items 64)
                  :max_string (or config.max_string 4000)
                  :redact {}}]
      (assert (and (= (type policy.max_string) :number) (> policy.max_string 0))
              "messages.max_string must be positive")
      (each [_ key (ipairs (or config.redact_keys
                               [:authorization
                                :api_key
                                :password
                                :secret
                                :token]))]
        (tset policy.redact (: (tostring key) :lower) true))
      (table.insert declarations
                    (let [definition {:action :toggle_verbose
                                      :context :global
                                      :default [:alt+t]}]
                      {:catalog :keybindings
                       :id (.. (. definition :context) "/"
                               (. definition :action))
                       :value definition}))
      (table.insert declarations
                    (let [definition {:action :transcript_up
                                      :context :global
                                      :default [:page_up :alt+k]}]
                      {:catalog :keybindings
                       :id (.. (. definition :context) "/"
                               (. definition :action))
                       :value definition}))
      (table.insert declarations
                    (let [definition {:action :transcript_down
                                      :context :global
                                      :default [:page_down :alt+j]}]
                      {:catalog :keybindings
                       :id (.. (. definition :context) "/"
                               (. definition :action))
                       :value definition}))
      (table.insert declarations
                    (let [definition {:hotkey {:action :toggle_verbose
                                               :context :global}
                                      :icon "≡"
                                      :id :transcript-detail
                                      :label :detail
                                      :query [:messages/detail-indicator]}]
                      {:catalog :indicators
                       :id (. definition :id)
                       :value definition}))
      (table.insert declarations
                    (let [definition {:id :messages/detail-indicator
                                      :inputs [[:db/path :messages :verbose]]
                                      :compute messages-detail-indicator-value}]
                      {:catalog :subscriptions
                       :id (. definition :id)
                       :value definition}))
      (table.insert declarations
                    {:catalog :events
                     :value {:event :app/start
                             :handler (fn [db] (initialize config db))}})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :messages/toggle-verbose
                             :handler toggle-detail}})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :messages/scroll :handler handle-scroll}})
      (let [scroll-inputs {:wheel_up (fn [] 3)
                           :wheel_down (fn [] -3)
                           :transcript_up (fn [cofx]
                                            (math.max 1
                                                      (math.floor (/ cofx.terminal.lines
                                                                     2))))
                           :transcript_down (fn [cofx]
                                              (- (math.max 1
                                                           (math.floor (/ cofx.terminal.lines
                                                                          2)))))}]
        (table.insert declarations
                      (let [definition {:id :messages/global-keys
                                        :event :terminal/input
                                        :priority 400
                                        :context [:db/path]
                                        :resolve (fn [db event cofx]
                                                   (terminal-input scroll-inputs
                                                                   db event cofx))}]
                        {:catalog :routes
                         :id (. definition :id)
                         :value definition}))
        (table.insert declarations
                      (let [definition {:binding {:action :toggle_verbose
                                                  :context :global}
                                        :event {:type :messages/toggle-verbose}
                                        :id :transcript.detail
                                        :label "Toggle transcript detail"}]
                        {:catalog :actions
                         :id (. definition :id)
                         :value definition}))
        (table.insert declarations
                      (let [definition {:binding {:action :transcript_up
                                                  :context :global}
                                        :event {:delta 10
                                                :type :messages/scroll}
                                        :id :transcript.up
                                        :label "Scroll transcript up"}]
                        {:catalog :actions
                         :id (. definition :id)
                         :value definition}))
        (table.insert declarations
                      (let [definition {:binding {:action :transcript_down
                                                  :context :global}
                                        :event {:delta (- 10)
                                                :type :messages/scroll}
                                        :id :transcript.down
                                        :label "Scroll transcript down"}]
                        {:catalog :actions
                         :id (. definition :id)
                         :value definition}))
        (table.insert declarations
                      {:catalog :selection-sources
                       :id :transcript
                       :value {:documents documents :layout document-layout}})
        (table.insert declarations
                      {:catalog :services :id :transcript.state :value state})
        (table.insert declarations
                      {:catalog :projections
                       :id :transcript.project
                       :value {:inputs projection-inputs
                               :render (fn [db render-context]
                                         (project markdown config db
                                                  render-context))}})
        (table.insert declarations
                      {:catalog :services
                       :id :transcript.viewport
                       :value viewport})
        (table.insert declarations
                      {:catalog :services :id :transcript.blocks :value blocks})
        (table.insert declarations
                      {:catalog :services
                       :id :transcript.window
                       :value transcript-window})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/reset :handler reset}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/response-start
                               :handler start-response}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/block-start
                               :handler start-block}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/block-delta
                               :handler (fn [db event]
                                          (update-block (misa.catalog :transcript-deltas)
                                                        policy db event))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/block-end
                               :handler (fn [db event]
                                          (finish-streaming-block policy db
                                                                  event))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/response-end
                               :handler (fn [db event cofx]
                                          (finish-response noninteractive-commit
                                                           markdown policy db
                                                           event cofx))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/response-interrupted
                               :handler (fn [db event cofx]
                                          (interrupt-response policy db event
                                                              cofx))}})
        (each [_ kind (ipairs [:user :harness])]
          (table.insert declarations
                        {:catalog :events
                         :value {:event (.. :transcript/ kind)
                                 :handler (fn [db event cofx]
                                            (append-message noninteractive-commit
                                                            markdown kind db
                                                            event cofx))}}))
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/tool-start
                               :handler start-tool}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/tool-result
                               :handler (fn [db event cofx]
                                          (finish-tool db event cofx))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/tool-summary
                               :handler summarize-tool}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/tool-call
                               :handler (fn [db event cofx]
                                          (append-tool policy db event cofx))}})
        ;; Compatibility completion input for custom agents. It is normalized once
        ;; into the same response/block lifecycle rather than maintained as a shadow.
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/assistant
                               :handler (fn [db event cofx]
                                          (append-assistant noninteractive-commit
                                                            markdown policy db
                                                            event cofx))}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :transcript/interrupted
                               :handler append-interrupted}})
        (table.insert declarations
                      {:catalog :events
                       :value {:event :runtime/dispatch-limit
                               :handler runtime-dispatch-limit}})
        (definitions.build :messages
          declarations
          {:transcript-presentations projectors
           :transcript-deltas delta-handlers
           :validators {:transcript-presentations (fn [_ handler]
                                                    (assert (= (type handler)
                                                               :function)
                                                            "presentation must be a function"))
                        :transcript-deltas (fn [_ handler]
                                             (assert (= (type handler)
                                                        :function)
                                                     "delta handler must be a function"))}})))))

{:build build}
