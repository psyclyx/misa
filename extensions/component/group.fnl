;; A quiet boundary around related content. The boundary accepts ordinary labels
;; and typed facts; transcript roles adapt response metadata to that vocabulary.
(fn boundary [model context]
  (local columns (math.max 1 (or context.columns 80)))
  (local parts [{:text "──" :style :dim}])
  (fn append [spans]
    (table.insert parts {:text "  " :style :dim})
    (each [_ part (ipairs spans)]
      (table.insert parts (misa.patch part {:style :dim :source false}))))
  (when model.label (append [{:text model.label}]))
  (each [_ fact (ipairs (or model.facts []))]
    (append (misa.render_value fact context)))
  (local lines (misa.layout.wrap_spans [{:spans parts}] columns))
  (local last (. lines (length lines)))
  (when last
    (local width (misa.layout.width (table.concat (icollect [_ part (ipairs last.spans)] part.text))))
    (when (< width columns)
      (table.insert last.spans {:text (.. " " (string.rep "─" (math.max 0 (- columns width 1))))
                               :style :dim :source false})))
  {: lines})

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
    (table.insert facts (if (or model.cost.pending (and model.cost.unknown (= model.cost.amount 0)))
                           {:type :sequence :values [{:type :text :value "cost "} model.cost]}
                           model.cost)))
  (context.render_child :group.boundary {: facts} context))

{:setup (fn []
          {:fx [{:type :register/component :id :default.group.boundary :value {:render boundary}}
                {:type :register/component :id :default.transcript.group_header :value {:render header :compose true}}
                {:type :register/component :id :default.transcript.group_footer :value {:render footer :compose true}}]})}
