;; Real model catalogue and default model picker; only provider I/O is omitted.
{:setup (fn [context]
          (local count context.config.benchmark.models)
          (local effects [])
          (for [index 1 count]
            (local model (string.format "model-%05d" index))
            (table.insert effects {:type :register/model
                                   :value {:id (.. :bench/ model) : model :provider :bench
                                           :label (.. "Benchmark " model)
                                           :context_window 200000}}))
          (table.insert effects {:type :register/event :name :app/start
                                 :handler (fn [] {:fx [{:type :dispatch :event {:type :model/picker-open}}]})})
          (table.insert effects {:type :register/event-route
                                 :value {:id :bench/quit :event :terminal/input :priority 2000
                                         :context [:db/path]
                                         :resolve (fn [_ event]
                                                    (when (and (= event.kind :alt) (= event.text :q))
                                                      {:type :bench/quit}))}})
          (table.insert effects {:type :register/event :name :bench/quit
                                 :handler (fn [] {:fx [{:type :app/quit}]})})
          {:fx effects})}
