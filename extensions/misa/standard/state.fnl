;; What each top-level state root is for, declared once so the invariants and the
;; later namespace moves have a checklist. See docs/architecture.md sections 4
;; and 10.
;;
;; `owner` says who may write a root:
;;
;; * `:kernel` owns durable session facts: the log and what the fold derives from
;;   it (history, the attempt ledger, the journal cursor, queueing).
;; * `:policy` owns nothing durable by design; a policy that seems to need state
;;   gets it from a kernel fact or a derived check instead.
;; * `:observation` describes the world and carries a check at the point of use.
;; * `:presentation` is render state and user input.
;; * `:external` is durable but outside the log: preferences, credentials, blobs.
;;
;; `lifetime` says how it survives:
;;
;; * `:log` durable facts, `:fold` a pure projection of them, `:observation`
;;   world-derived with a check, `:ephemeral` in-flight or interaction state,
;;   `:external` a separate durable store.
;;
;; A root may hold more than one lifetime until the moves in section 8 split it
;; (the model catalogue and the model selection share `models`, for instance), so
;; the declaration names the owner and the dominant lifetime.

{:state {:agent {:lifetime :fold :owner :kernel}
         :auth_startup {:lifetime :ephemeral :owner :kernel}
         :conversation {:lifetime :fold :owner :kernel}
         :costs {:lifetime :fold :owner :kernel}
         :queue {:lifetime :ephemeral :owner :kernel}
         :compaction {:lifetime :ephemeral :owner :policy}
         :request_options {:lifetime :fold :owner :policy}
         :model_discovery {:lifetime :observation :owner :observation}
         :models {:lifetime :observation :owner :observation}
         :provider_availability {:lifetime :observation :owner :observation}
         :providers {:lifetime :observation :owner :observation}
         :usage {:lifetime :observation :owner :observation}
         :preferences {:lifetime :external :owner :external}
         :animations {:lifetime :ephemeral :owner :presentation}
         :choice_commands {:lifetime :ephemeral :owner :presentation}
         :clipboard {:lifetime :ephemeral :owner :presentation}
         :components {:lifetime :ephemeral :owner :presentation}
         :dialog {:lifetime :ephemeral :owner :presentation}
         :editing {:lifetime :ephemeral :owner :presentation}
         :editor {:lifetime :ephemeral :owner :presentation}
         :history {:lifetime :ephemeral :owner :presentation}
         :hover_action {:lifetime :ephemeral :owner :presentation}
         :hover_link {:lifetime :ephemeral :owner :presentation}
         :images {:lifetime :ephemeral :owner :presentation}
         :link_sequence {:lifetime :ephemeral :owner :presentation}
         :messages {:lifetime :fold :owner :presentation}
         :omnipicker_sequence {:lifetime :ephemeral :owner :presentation}
         :picker {:lifetime :ephemeral :owner :presentation}
         :selection {:lifetime :ephemeral :owner :presentation}
         :syntax {:lifetime :fold :owner :presentation}
         :themes {:lifetime :ephemeral :owner :presentation}}
 :validators {:state (fn [id value]
                       "Every declared state root names a known owner, a known lifetime, and nothing
     else; an undeclared root is simply not listed here yet."
                       (assert (= (type value) :table)
                               (.. "state root needs a declaration: " id))
                       (let [owners {:external true
                                     :kernel true
                                     :observation true
                                     :policy true
                                     :presentation true}
                             lifetimes {:ephemeral true
                                        :external true
                                        :fold true
                                        :log true
                                        :observation true}]
                         (assert (. owners value.owner)
                                 (.. "state root " id " has an unknown owner: "
                                     (tostring value.owner)))
                         (assert (. lifetimes value.lifetime)
                                 (.. "state root " id
                                     " has an unknown lifetime: "
                                     (tostring value.lifetime)))
                         (each [field _ (pairs value)]
                           (assert (or (= field :owner) (= field :lifetime))
                                   (.. "state root " id
                                       " has an unknown field: "
                                       (tostring field))))))}}
