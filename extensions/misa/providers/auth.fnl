;; Authentication commands, account selection, and startup status.
;;
;; A provider stores several named accounts. `default` is the account a plain
;; `/login PROVIDER` writes, and the account a provider's requests use is the
;; one login or `/account` most recently selected.
(local max-account-length 64)

(fn valid-account? [name]
  "Mirror the native account-name contract for early feedback."
  (and (= (type name) :string) (> (length name) 0)
       (<= (length name) max-account-length)
       (let [(cleaned) (name:gsub "[%w%._%-]" "")]
         (= cleaned ""))))

(fn auth-effect [action provider account id completion]
  {: account
   : action
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
      (let [effect (auth-effect :status provider nil provider.model_provider
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
  "Track authentication and model availability."
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
  "Advance startup after provider discovery."
  (let [startup db.auth_startup]
    (when (and startup (. startup.pending_discovery event.provider))
      (startup-progress startup
                        {:pending_discovery {event.provider misa.delete}} []))))

(fn command-request [item event]
  "Resolve a command's provider and account, or describe a usage problem."
  (let [arguments (or (and (= (type event.arguments) :string) event.arguments)
                      "")
        tokens (icollect [token (arguments:gmatch "%S+")] token)]
    (if (or (= (length tokens) 0) (> (length tokens) 2))
        {:id item.action
         :message (.. "usage: " item.name
                      (if (= item.action :select) " <provider> <account>"
                          " <provider> [account]"))}
        (let [provider (. tokens 1)
              account (. tokens 2)
              declaration (misa.auth.provider provider)]
          (if (not declaration)
              {:id (.. item.action ":" provider)
               :message "unknown provider"
               : provider}
              (and account (not (valid-account? account)))
              {:id (.. item.action ":" provider ":" account)
               :message (.. "invalid account name: " account
                            " (letters, digits, '.', '_' and '-' only)")
               : provider}
              (and (= item.action :select) (not account))
              {:id (.. item.action ":" provider)
               :message (.. "usage: " item.name " <provider> <account>")
               : provider}
              {: account : declaration : provider})))))

(fn command [item _ event cofx]
  "Translate an authentication command into native effects."
  (let [request (command-request item event)]
    (if (not request.declaration)
        {:fx [{:event {:id request.id
                       :message request.message
                       :ok false
                       :provider request.provider
                       :type :auth/complete}
               :type :dispatch}]}
        (let [provider request.provider
              account request.account
              effects {}]
          (when cofx.terminal.interactive
            (tset effects (+ (length effects) 1)
                  {:event {:level :info
                           :text (.. item.action " " provider
                                     (if account (.. " " account) "") "…")
                           :type :transcript/harness}
                   :type :dispatch}))
          (tset effects (+ (length effects) 1)
                (auth-effect item.action request.declaration account
                             (.. item.action ":" provider
                                 (if account (.. ":" account) ""))))
          {:fx effects}))))

(fn interaction [db event]
  "Present an authentication interaction."
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
  "Translate an authentication dialog action."
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
  "Finish an authentication interaction."
  (var message (or event.message "authentication finished"))
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

{:startup start
 : provider-status
 : valid-account?
 : discovery-complete
 : command
 : interaction
 : dialog-action
 : complete}
