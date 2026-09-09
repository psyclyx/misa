(local fennel (require :fennel))
(set fennel.path (.. "extensions/?.fnl;extensions/?/init.fnl;" fennel.path))

(fn application [context]
  "Collect test declaration data and install it once."
  (let [config (or context {:argv [] :config {}})
        catalogs {}]
    (var order 0)

    (fn define [definitions]
      "Add catalogs with unique identities in fixture order."
      (set order (+ order 1))
      (each [kind entries (pairs definitions)]
        (when (not (. catalogs kind)) (tset catalogs kind {}))
        (each [id value (pairs entries)]
          (assert (= (. catalogs kind id) nil)
                  (.. "duplicate test definition: " kind "/" id))
          (let [entry (if (= kind :events)
                          (let [copy (collect [key value (pairs value)] key
                                       value)]
                            (set copy.priority (or value.priority order))
                            copy)
                          value)]
            (tset catalogs kind id entry))))
      definitions)

    {: define
     :include define
     :definitions catalogs
     :context config
     :add (fn [name] (define (require name)))
     :install (fn [options] (_G.misa._install catalogs (or options config)))}))

application
