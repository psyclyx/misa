(local fennel (require :fennel))
(local print print)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local queue (require :misa.editor.queue))
(local history (require :misa.editor.history))
(local preferences (require :misa.choices.preferences))
(local matching (require :misa.choices.matching))
(local options (require :misa.models.options))

;; Policy calls do not construct or install the standard application.
(let [original {:pending "first"
                :attachments [{:path "one.png"}]
                :sending false}
      combined (queue.append original "second" [{:path "two.png"}])
      delivered (queue.drain {:agent {:status :ready}} combined)]
  (assert (= original.pending "first"))
  (assert (= (length original.attachments) 1))
  (assert (= combined.pending "first\nsecond"))
  (assert (= (length combined.attachments) 2))
  (assert (= (. delivered.fx 1 :event :prompt) "first\nsecond"))
  (assert (= (. delivered.fx 2 :event :type) :queue/submission-settled))
  (assert (= delivered.patch.queue.sending true))
  (assert (= (queue.acknowledge {:queue combined
                                 :agent {:startup_prompt "auth"}})
             nil))
  (let [reserved (queue.drain {:agent {:status :ready}}
                              (misa.patch combined {:sending true}))]
    (assert (= reserved.fx nil))))

(let [busy {:agent {:status :streaming}
            :queue {:pending "queued" :attachments [] :sending false}}
      interrupted (queue.steer busy {:prompt "next"})
      restored (queue.take busy)]
  (assert (= interrupted.patch.queue.pending "queued\nnext"))
  (assert (= (. interrupted.fx 1 :event :type) :agent/cancel-active))
  (assert (= (. restored.fx 1 :event :text) "queued"))
  (assert (= restored.patch.queue.pending ""))
  (assert (= busy.queue.pending "queued")))

(let [entries ["one" "two" "three"]]
  (assert (= (table.concat (history.recent-entries entries 2 100) ",")
             "two,three"))
  (assert (= (table.concat (history.recent-entries entries 10 7) ",") "three"))
  (assert (= (length (history.recent-entries entries 10 4)) 0))
  (assert (= (length (history.recent-entries [] 2 4)) 0))
  (assert (= (length entries) 3)))

(let [valid {:clock 2
             :scopes {:models {:one {:favorite false :uses 1 :last 2}}}}]
  (assert (preferences.valid? valid))
  (assert (not (preferences.valid? {:clock 0
                                    :scopes {:models {:one {:favorite true
                                                            :uses 1}}}})))
  (assert (not (preferences.valid? {:clock 1
                                    :scopes {:models {:one {:favorite false
                                                            :uses 1
                                                            :last 2}}}})))
  (assert (not (preferences.valid? {:clock 0 :scopes {:models false}})))
  (assert (not (preferences.valid? {:clock (/ 0 0) :scopes {}})))
  (let [used (preferences.use {:preferences valid} :models :one)]
    (assert (= used.clock 3))
    (assert (= used.scopes.models.one.uses 2))
    (assert (= valid.clock 2))))

(let [source [{:value :beta :label "same"}
              {:value :alpha :label "same"}
              {:value :alpha :label "same"}]
      ranked (matching.choices source "same" (fn [item] item.label))]
  (assert (= (. ranked 1) (. source 2)))
  (assert (= (. ranked 2) (. source 3)))
  (assert (= (. ranked 3) (. source 1)))
  (assert (= (matching.score "absent" "same") nil)))

(let [model {:id :one
             :api {:request_options {:effort {:choices [:low :high]
                                              :default :low}}}}
      state (options.reconcile {:models {:selected :one :entries [model]}
                                :request_options {:configured {:effort :high}
                                                  :values {}}})]
  (assert (= state.values.effort :high))
  (assert (= (. (options.reconcile {:request_options {:model_id :old
                                                      :values {:effort :unsupported}}}
                                   model) :values :effort) :low)))

(print "feature policy seams passed")
