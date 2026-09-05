;; Authentication commands; secret handling and provider-specific flows remain native.

{:setup (fn []
          (local providers (misa.auth_providers))

          (fn provider-by-id [id]
            (each [_ provider (ipairs providers)]
              (when (= provider.id id) (lua "return provider")))
            nil)

          (fn auth-effect [action provider id]
            {: action
             :completion :auth/complete
             : id
             :interaction :auth/interaction
             :profile provider.profile
             :provider provider.id
             :strategy provider.strategy
             :type :auth/command})

          (fn model-provider-for [id]
            (each [_ provider (ipairs providers)]
              (when (= provider.id id)
                (let [___antifnl_rtn_1___ provider.model_provider]
                  (lua "return ___antifnl_rtn_1___"))))
            nil)

          (misa.reg_interceptor {:before (fn [tx]
                                           (when (and (= tx.event.type
                                                         :app/start)
                                                      (not tx.db.auth_startup))
                                             (set tx.db.auth_startup
                                                  {:pending_discovery 0
                                                   :pending_status (length providers)
                                                   :ready (= (length providers)
                                                             0)}))
                                           tx)
                                 :id :auth/startup-state})
          (misa.reg_event :app/start
                          (fn [db]
                            (local effects {})
                            (each [_ provider (ipairs providers)]
                              (local effect
                                     (auth-effect :status provider
                                                  provider.model_provider))
                              (set effect.completion :auth/provider-status)
                              (tset effects (+ (length effects) 1) effect))
                            (when (= (length providers) 0)
                              (tset effects (+ (length effects) 1)
                                    {:event {:type :auth/startup-ready}
                                     :type :dispatch}))
                            {: db :fx effects}))
          (misa.reg_event :auth/provider-status
                          (fn [db event]
                            (local startup
                                   (assert db.auth_startup
                                           "auth startup state is missing"))
                            (set startup.pending_status
                                 (- startup.pending_status 1))
                            (local available
                                   (and event.ok (= event.logged_in true)))
                            (local effects
                                   [{:event {: available
                                             :provider event.id
                                             :subscription_type event.subscription_type
                                             :type :models/provider-availability}
                                     :type :dispatch}])
                            (var declaration nil)
                            (each [_ provider (ipairs providers)]
                              (when (= provider.model_provider event.id)
                                (set declaration provider)
                                (lua :break)))
                            (when (and (and available declaration)
                                       declaration.discover_models)
                              (set startup.pending_discovery
                                   (+ startup.pending_discovery 1))
                              (tset effects (+ (length effects) 1)
                                    {:event {:provider event.id
                                             :type :models/discover}
                                     :type :dispatch}))
                            (when (and (= startup.pending_status 0)
                                       (= startup.pending_discovery 0))
                              (set startup.ready true)
                              (tset effects (+ (length effects) 1)
                                    {:event {:type :auth/startup-ready}
                                     :type :dispatch}))
                            {: db :fx effects}))
          (misa.reg_event :models/discovery-complete
                          (fn [db]
                            (local startup db.auth_startup)
                            (if (or (not startup) startup.ready) nil
                                (do
                                  (set startup.pending_discovery
                                       (math.max 0
                                                 (- startup.pending_discovery 1)))
                                  (if (and (= startup.pending_status 0)
                                           (= startup.pending_discovery 0))
                                      (do
                                        (set startup.ready true)
                                        {: db
                                         :fx [{:event {:type :auth/startup-ready}
                                               :type :dispatch}]})
                                      {: db})))))
          (each [_ command (ipairs [{:action :login
                                     :description "Log in to a provider"
                                     :name :/login}
                                    {:action :logout
                                     :description "Log out of a provider"
                                     :name :/logout}
                                    {:action :status
                                     :description "Show provider login state"
                                     :name :/status}])]
            (local item command)
            (local event-type (.. :auth/ item.action))
            (misa.reg_command {:choice_purpose :auth
                               :completion :auth-provider
                               :description item.description
                               :event event-type
                               :name item.name})
            (misa.reg_event event-type
                            (fn [_ event cofx]
                              (local provider
                                     (or (and (= (type event.arguments) :string)
                                              (event.arguments:match "^%s*(%S+)%s*$"))
                                         nil))
                              (if (not provider)
                                  {:fx [{:event {:id item.action
                                                 :message (.. "usage: "
                                                              item.name
                                                              " <provider>")
                                                 :ok false
                                                 :type :auth/complete}
                                         :type :dispatch}]}
                                  (do
                                    (local effects {})
                                    (when cofx.terminal.interactive
                                      (tset effects (+ (length effects) 1)
                                            {:event {:level :info
                                                     :text (.. item.action " "
                                                               provider "…")
                                                     :type :transcript/harness}
                                             :type :dispatch}))
                                    (local declaration
                                           (provider-by-id provider))
                                    (if (not declaration)
                                        (tset effects (+ (length effects) 1)
                                              {:event {:id (.. item.action ":"
                                                               provider)
                                                       :message "unknown provider"
                                                       :ok false
                                                       : provider
                                                       :type :auth/complete}
                                               :type :dispatch})
                                        (tset effects (+ (length effects) 1)
                                              (auth-effect item.action
                                                           declaration
                                                           (.. item.action ":"
                                                               provider))))
                                    {:fx effects})))))
          (misa.reg_event :auth/interaction
                          (fn [db event]
                            (local dialog
                                   {:actions event.actions
                                    :cancellable event.cancellable
                                    :code event.code
                                    :completion :auth/dialog-action
                                    :correlation (or event.correlation event.id)
                                    :hints event.hints
                                    :id event.id
                                    :input event.input
                                    :kind event.kind
                                    :message event.message
                                    :progress event.progress
                                    :protected event.protected
                                    :title event.title
                                    :type (or (and db.dialog :dialog/update)
                                              :dialog/open)
                                    :url event.url})
                            {: db
                             :fx [{:event dialog :type :dispatch}
                                  {:type :terminal/read}]}))
          (misa.reg_event :auth/dialog-action
                          (fn [db event]
                            (if event.cancelled
                                {: db
                                 :fx [{:id event.id :type :operation/cancel}
                                      {:type :terminal/read}]}
                                (if event.protected
                                    {: db :fx [{:type :terminal/read}]}
                                    {: db
                                     :fx [{:action event.action
                                           :correlation event.correlation
                                           :id event.id
                                           :type :auth/respond
                                           :value event.value}]}))))
          (misa.reg_event :auth/complete
                          (fn [db event]
                            (when (and db.dialog (= db.dialog.id event.id))
                              (set db.dialog nil))
                            (var message event.message)
                            (when (and event.subscription_type
                                       (not= event.subscription_type
                                             misa.json_null))
                              (set message
                                   (.. message " (" event.subscription_type ")")))
                            (local effects
                                   [{:event {:level (or (and event.ok :info)
                                                        :error)
                                             :text message
                                             :type :transcript/harness}
                                     :type :dispatch}])
                            (local provider (model-provider-for event.provider))
                            (when (and provider event.ok)
                              (tset effects (+ (length effects) 1)
                                    {:event {:available (= event.logged_in true)
                                             : provider
                                             :subscription_type event.subscription_type
                                             :type :models/provider-availability}
                                     :type :dispatch})
                              (when (= event.logged_in true)
                                (tset effects (+ (length effects) 1)
                                      {:event {: provider
                                               :type :models/discover}
                                       :type :dispatch})))
                            (tset effects (+ (length effects) 1)
                                  {:event {:type :auth/ready} :type :dispatch})
                            {:fx effects}))
          (misa.reg_event :auth/ready
                          (fn [db]
                            {: db :fx [{:type :terminal/read}]}))
          nil)}

