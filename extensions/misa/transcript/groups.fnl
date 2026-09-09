(fn header [model context]
  "Render the beginning of a transcript group."
  {:lines []})

(fn footer [model context]
  "Render timing and usage facts at the end of a transcript group."
  (let [facts []]
    (when model.started_wall_ms
      (table.insert facts {:type :timestamp :value model.started_wall_ms}))
    (when (or (= model.status :streaming) (= model.status :running))
      (table.insert facts {:type :text :value model.status}))
    (when (= model.status :interrupted)
      (table.insert facts {:type :text :value :interrupted}))
    (when model.elapsed_ms
      (table.insert facts {:type :duration :value model.elapsed_ms}))
    (when model.tokens_per_second
      (table.insert facts {:type :rate
                           :value model.tokens_per_second
                           :unit :tok}))
    (when model.cost
      (table.insert facts (if (or model.cost.pending
                                  (and model.cost.unknown
                                       (= model.cost.amount 0)))
                              {:type :sequence
                               :values [{:type :text :value "cost "}
                                        model.cost]}
                              model.cost)))
    (context.render_child :group.boundary {: facts} context)))

{: footer : header}
