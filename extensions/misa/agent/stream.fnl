(local definitions (require :misa.definitions))

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

    (fn plain-text [effect]
      (let [event effect.event
            delta (and (= (type event) :table) event.delta)]
        (when (and (= effect.type :dispatch) event
                   (= event.type :agent/stream-delta) (= (type delta) :table)
                   (or (= delta.type :text) (= delta.type :thinking))
                   (= (type delta.text) :string))
          (each [key (pairs effect)]
            (when (not (or (= key :type) (= key :event)))
              (lua "return false")))
          (each [key (pairs event)]
            (when (not (or (= key :type) (= key :id) (= key :delta)))
              (lua "return false")))
          (each [key (pairs delta)]
            (when (not (or (= key :type) (= key :text)))
              (lua "return false")))
          true)))

    (each [_ effect (ipairs effects)]
      (if (plain-text effect)
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

(fn build []
  "Install the standard stream normalization policy."
  (definitions.build :stream
    [{:catalog :services :id :stream.effects :value stream-effects}]
    {}))

{: build}
