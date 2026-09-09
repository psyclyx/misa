(fn allowed-keys? [value allowed]
  (accumulate [valid true key (pairs value)]
    (and valid (= (. allowed key) true))))

(fn plain-text? [effect]
  (let [event effect.event
        delta (and (= (type event) :table) event.delta)]
    (and (= effect.type :dispatch) event (= event.type :agent/stream-delta)
         (= (type delta) :table)
         (or (= delta.type :text) (= delta.type :thinking))
         (= (type delta.text) :string)
         (allowed-keys? effect {:type true :event true})
         (allowed-keys? event {:type true :id true :delta true})
         (allowed-keys? delta {:type true :text true}))))

(fn stream-effects [effects]
  "Combine adjacent plain deltas in one batch, preserving request and effect boundaries."
  (let [result []]
    (var (first chunks) nil)

    (fn flush []
      (when first
        (table.insert result
                      (if (= (length chunks) 1) first
                          (misa.patch first
                                      {:event {:delta {:text (table.concat chunks)}}})))
        (set (first chunks) (values nil nil))))

    (each [_ effect (ipairs effects)]
      (if (plain-text? effect)
          (do
            (when (and first
                       (or (not= first.event.id effect.event.id)
                           (not= first.event.delta.type effect.event.delta.type)))
              (flush))
            (when (not first)
              (set (first chunks) (values effect [])))
            (table.insert chunks effect.event.delta.text))
          (do
            (flush)
            (table.insert result effect))))
    (flush)
    result))

{:effects stream-effects}
