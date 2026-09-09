(local definitions (require :misa.definitions))

;; Real model catalogue and default model picker; only provider I/O is omitted.
(fn [context]
          (local count context.config.benchmark.models)
          (local effects [])
          (for [index 1 count]
            (local model (string.format "model-%05d" index))
            (table.insert effects (let [definition {:id (.. :bench/ model) : model :provider :bench
                                           :label (.. "Benchmark " model)
                                           :context_window 200000}] {:catalog :models :id (. definition :id) :value definition})))
          (table.insert effects {:catalog :events  :value {:event :app/start :handler (fn [] {:fx [{:type :dispatch :event {:type :model/picker-open}}]})}})
          (table.insert effects (let [definition {:id :bench/quit :event :terminal/input :priority 2000
                                         :context [:db/path]
                                         :resolve (fn [_ event]
                                                    (when (and (= event.kind :alt) (= event.text :q))
                                                      {:type :bench/quit}))}] {:catalog :routes :id (. definition :id) :value definition}))
          (table.insert effects {:catalog :events  :value {:event :bench/quit :handler (fn [] {:fx [{:type :app/quit}]})}})
          (definitions.build :benchmarks.picker-key-repeat effects {}))
