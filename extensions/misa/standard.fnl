(fn module-before? [a b]
  (let [left (or a.module.priority 0)
        right (or b.module.priority 0)]
    (if (= left right) (< a.id b.id) (< left right))))

(fn construct [module config]
  (assert (not (and module.source module.build))
          "module must choose source or build")
  (when module.source
    (assert (= (type module.source) :string) "module source must be a name"))
  (let [implementation (if module.source (require module.source)
                           (assert module.build
                                   "module requires source or a pure constructor"))]
    (if (= (type implementation) :function) (implementation {: config})
        implementation.build (implementation.build {: config})
        (assert implementation.definitions
                "module requires build or definitions"))))

(fn ordered-definitions [definitions priority]
  (let [events (collect [id event (pairs (or definitions.events {}))]
                 id
                 (if (= event misa.delete) misa.delete
                     (let [ordered (collect [key value (pairs event)] key value)]
                       (set ordered.priority (+ priority (or event.priority 0)))
                       ordered)))
        owned (collect [key value (pairs definitions)] key value)]
    (set owned.events events)
    owned))

(fn application [spec]
  "Compile an application specification into settings and named definitions.

Module builders produce declarations without installing them."
  (let [config (or spec.config {})
        modules (icollect [id module (pairs (or spec.modules {}))]
                  {: id : module})]
    (table.sort modules module-before?)
    (let [applications (icollect [_ entry (ipairs modules)]
                         (let [definitions (construct entry.module config)]
                           (assert (= (type definitions) :table)
                                   (.. "module returned no definitions: "
                                       entry.id))
                           {:definitions (ordered-definitions definitions
                                                              (or entry.module.priority
                                                                  0))}))]
      (table.insert applications
                    {: config :definitions (or spec.definitions {})})
      (let [composed (misa.compose applications)]
        {:config composed.config :definitions composed.definitions}))))

{: application :default (require :misa.default)}
