(local fennel (require :fennel))
(set fennel.path (.. "extensions/?.fnl;extensions/?/init.fnl;" fennel.path))

(fn application [context]
  "Compose test declarations and install the finished application once."
  (local config (or context {:argv [] :config {}}))
  (local catalogs {})
  (var order 0)

  (fn define [definitions]
    (set order (+ order 1))
    (each [kind entries (pairs definitions)]
      (when (not (. catalogs kind)) (tset catalogs kind {}))
      (each [id value (pairs entries)]
        (assert (= (. catalogs kind id) nil)
                (.. "duplicate test definition: " kind "/" id))
        (local entry (if (= kind :events)
                         (let [copy (collect [key value (pairs value)] key
                                      value)]
                           (set copy.priority (or value.priority order))
                           copy)
                         value))
        (tset catalogs kind id entry)))
    definitions)

  (fn add-module [module options]
    "Add a module's pure declarations to this test application."
    (define (if (= (type module) :function) (module (or options config))
                module.build (module.build (or options config))
                module.definitions)))

  {: define
   :include add-module
   :definitions catalogs
   :context config
   :add (fn [name]
          (add-module (require name)))
   :install (fn [options] (_G.misa._install catalogs (or options config)))})

application
