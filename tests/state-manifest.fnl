;; The state manifest. `misa.standard.state` declares what each top-level state
;; root is for, and the install validator refuses a declaration that names an
;; unknown owner, an unknown lifetime, or an extra field.
;;
;; These assertions are the checklist the namespace moves in
;; docs/architecture.md follow: a root that moves between layers changes here and
;; in the test that pins it. They do not yet prove coverage — a new root that
;; nobody declares passes unnoticed — so the manifest is a maintained list, not an
;; enforced one, until a full-application harness can observe every root an
;; installed profile writes.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local stock (require :tests.stock))

(app.define (. stock :misa.state))
(app.install)

(local manifest (misa.catalog :state))
(fn declared [root]
  (let [entry (. manifest root)]
    (assert entry (.. "state root is not declared: " (tostring root)))
    entry))

;; Ownership: the kernel owns durable session facts; a policy owns no state of
;; its own; an observation describes the world; presentation is render state and
;; input; external is durable but outside the log.
(each [root expected (pairs {:agent :kernel
                             :conversation :kernel
                             :costs :kernel
                             :queue :kernel
                             :compaction :policy
                             :request_options :policy
                             :model_discovery :observation
                             :models :observation
                             :providers :observation
                             :usage :observation
                             :dialog :presentation
                             :editor :presentation
                             :messages :presentation
                             :selection :presentation
                             :syntax :presentation
                             :preferences :external})]
  (assert (= (. (declared root) :owner) expected)
          (.. "state root " root " is not owned by " (tostring expected))))

;; Lifetime: durable facts fold from the log, in-flight work and interaction
;; state are ephemeral, preferences live outside the log.
(assert (= (. (declared :agent) :lifetime) :fold))
(assert (= (. (declared :conversation) :lifetime) :fold))
(assert (= (. (declared :compaction) :lifetime) :ephemeral))
(assert (= (. (declared :editor) :lifetime) :ephemeral))
(assert (= (. (declared :models) :lifetime) :observation))
(assert (= (. (declared :preferences) :lifetime) :external))

;; Every root declares both fields and nothing else.
(each [root entry (pairs manifest)]
  (assert (and (. entry :owner) (. entry :lifetime))
          (.. "state root " root " is missing a declaration"))
  (each [field _ (pairs entry)]
    (assert (or (= field :owner) (= field :lifetime))
            (.. "state root " root " declares an unknown field "
                (tostring field)))))

;; The validator refuses what the manifest forbids, so a hand-written
;; declaration cannot quietly acquire a new owner.
(local validate (. (misa.catalog :validators) :state))
(assert (= (type validate) :function) "the state catalog has no validator")
(local nonsense {:bogus_owner {:lifetime :fold :owner :nope}
                 :bogus_lifetime {:lifetime :nope :owner :kernel}
                 :extra_field {:extra true :lifetime :fold :owner :kernel}
                 :not_a_table :fold})
(each [name value (pairs nonsense)]
  (assert (not (pcall validate name value))
          (.. "the state validator accepted " (tostring name))))

(output "state manifest contracts passed\n")
