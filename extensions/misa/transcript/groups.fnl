(local definitions (require :misa.definitions))

(fn header [model context]
  {:lines []})

(fn footer [model context]
  (local facts [])
  (when model.started_wall_ms
    (table.insert facts {:type :timestamp :value model.started_wall_ms}))
  (when (or (= model.status :streaming) (= model.status :running))
    (table.insert facts {:type :text :value model.status}))
  (when (= model.status :interrupted)
    (table.insert facts {:type :text :value :interrupted}))
  (when model.elapsed_ms
    (table.insert facts {:type :duration :value model.elapsed_ms}))
  (when model.tokens_per_second
    (table.insert facts {:type :rate :value model.tokens_per_second :unit :tok}))
  (when model.cost
    (table.insert facts (if (or model.cost.pending
                                (and model.cost.unknown (= model.cost.amount 0)))
                            {:type :sequence
                             :values [{:type :text :value "cost "} model.cost]}
                            model.cost)))
  (context.render_child :group.boundary {: facts} context))

(fn []
  "Declare transcript turn boundaries and metadata."
  (definitions :transcript.groups
    [{:catalog :components
      :id :default.transcript.group_header
      :value {:render header :compose true}}
     {:catalog :components
      :id :default.transcript.group_footer
      :value {:render footer :compose true}}]
    {}))
