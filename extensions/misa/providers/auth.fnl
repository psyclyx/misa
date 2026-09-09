(local definitions (require :misa.definitions))

(fn auth-effect [action provider id completion]
  {: action
   :completion (or completion :auth/complete)
   : id
   :interaction :auth/interaction
   :profile provider.profile
   :provider provider.id
   :strategy provider.strategy
   :type :auth/command})

(fn startup-progress [startup patch effects]
  (let [next-state (misa.patch startup patch)
        ready (and (= (next next-state.pending_status) nil)
                   (= (next next-state.pending_discovery) nil))]
    (when ready
      (table.insert effects {:event {:type :auth/startup-ready}
                             :type :dispatch}))
    {:patch {:auth_startup (misa.replace (misa.patch next-state {: ready}))}
     :fx effects}))

(fn start [providers db]
  "Request account status for the configured providers."
  (let [effects {}
        pending {}]
    (each [_ provider (ipairs providers)]
      (tset pending provider.model_provider true)
      (let [effect (auth-effect :status provider provider.model_provider
                                :auth/provider-status)]
        (tset effects (+ (length effects) 1) effect)))
    (when (= (length providers) 0)
      (tset effects (+ (length effects) 1)
            {:event {:type :auth/startup-ready} :type :dispatch}))
    {:patch (when (not db.auth_startup)
              {:auth_startup (misa.replace {:pending_discovery {}
                                            :pending_status pending
                                            :ready (= (length providers) 0)})})
     :fx effects}))

(fn provider-status [db event]
  (let [startup (assert db.auth_startup "auth startup state is missing")]
    (when (. startup.pending_status event.id)
      (let [available (and event.ok (= event.logged_in true))
            effects [{:event {: available
                              :provider event.id
                              :subscription_type event.subscription_type
                              :type :models/provider-availability}
                      :type :dispatch}]
            declaration (misa.auth.for-model event.id)
            discover (and available declaration declaration.discover_models)]
        (when discover
          (tset effects (+ (length effects) 1)
                {:event {:provider event.id :type :models/discover}
                 :type :dispatch}))
        (startup-progress startup
                          {:pending_status {event.id misa.delete}
                           :pending_discovery (when discover
                                                {event.id true})}
                          effects)))))

(fn discovery-complete [db event]
  (let [startup db.auth_startup]
    (when (and startup (. startup.pending_discovery event.provider))
      (startup-progress startup
                        {:pending_discovery {event.provider misa.delete}} []))))

(fn command [item _ event cofx]
  "Translate an authentication command into native effects."
  (let [provider (or (and (= (type event.arguments) :string)
                          (event.arguments:match "^%s*(%S+)%s*$"))
                     nil)]
    (if (not provider)
        {:fx [{:event {:id item.action
                       :message (.. "usage: " item.name " <provider>")
                       :ok false
                       :type :auth/complete}
               :type :dispatch}]}
        (let [effects {}]
          (when cofx.terminal.interactive
            (tset effects (+ (length effects) 1)
                  {:event {:level :info
                           :text (.. item.action " " provider "…")
                           :type :transcript/harness}
                   :type :dispatch}))
          (let [declaration (misa.auth.provider provider)]
            (if (not declaration)
                (tset effects (+ (length effects) 1)
                      {:event {:id (.. item.action ":" provider)
                               :message "unknown provider"
                               :ok false
                               : provider
                               :type :auth/complete}
                       :type :dispatch})
                (tset effects (+ (length effects) 1)
                      (auth-effect item.action declaration
                                   (.. item.action ":" provider))))
            {:fx effects})))))

(fn interaction [db event]
  (let [dialog {:actions event.actions
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
                :type (or (and db.dialog :dialog/update) :dialog/open)
                :url event.url}]
    {:fx [{:event dialog :type :dispatch} {:type :terminal/read}]}))

(fn dialog-action [db event]
  (if event.cancelled
      {:fx [{:id event.id :type :operation/cancel} {:type :terminal/read}]}
      (if event.protected
          {:fx [{:type :terminal/read}]}
          {:fx [{:action event.action
                 :correlation event.correlation
                 :id event.id
                 :type :auth/respond
                 :value event.value}]})))

(fn complete [db event]
  (var message event.message)
  (when (and event.subscription_type
             (not= event.subscription_type misa.json-null))
    (set message (.. message " (" event.subscription_type ")")))
  (let [effects [{:event {:level (or (and event.ok :info) :error)
                          :text message
                          :type :transcript/harness}
                  :type :dispatch}]
        declaration (misa.auth.provider event.provider)
        provider (and declaration declaration.model_provider)]
    (when (and provider event.ok)
      (tset effects (+ (length effects) 1)
            {:event {:available (= event.logged_in true)
                     : provider
                     :subscription_type event.subscription_type
                     :type :models/provider-availability}
             :type :dispatch})
      (when (= event.logged_in true)
        (tset effects (+ (length effects) 1)
              {:event {: provider :type :models/discover} :type :dispatch})))
    (tset effects (+ (length effects) 1)
          {:event {:type :auth/ready} :type :dispatch})
    {:patch {:dialog (when (and db.dialog (= db.dialog.id event.id))
                       misa.delete)}
     :fx effects}))

(fn build []
  "Build the declarations for auth."
  (let [declarations []
        providers (misa.auth.providers)]
    (table.insert declarations
                  {:catalog :events
                   :value {:event :app/start
                           :handler (fn [db]
                                      (start providers db))}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :auth/provider-status
                           :handler provider-status}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :models/discovery-complete
                           :handler discovery-complete}})
    (each [_ item (ipairs [{:action :login
                            :description "Log in to a provider"
                            :name :/login}
                           {:action :logout
                            :description "Log out of a provider"
                            :name :/logout}
                           {:action :status
                            :description "Show provider login state"
                            :name :/status}])]
      (let [event-type (.. :auth/ item.action)]
        (table.insert declarations
                      (let [definition {:choice_purpose :auth
                                        :completion :auth-provider
                                        :description item.description
                                        :event event-type
                                        :name item.name}]
                        {:catalog :commands
                         :id (. definition :name)
                         :value definition}))
        (table.insert declarations
                      {:catalog :events
                       :value {:event event-type
                               :handler (fn [_ event cofx]
                                          (command item _ event cofx))}})))
    (table.insert declarations
                  {:catalog :events
                   :value {:event :auth/interaction :handler interaction}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :auth/dialog-action :handler dialog-action}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :auth/complete :handler complete}})
    (table.insert declarations
                  {:catalog :events
                   :value {:event :auth/ready
                           :handler (fn [db]
                                      {:fx [{:type :terminal/read}]})}})
    (definitions.build :auth declarations {})))

{:build build :startup start :command command}
