(fn application [spec]
  "Compile an application specification into settings and named definitions.

Module constructors only produce declarations. This function installs nothing."
  (local modules (icollect [id module (pairs (or spec.modules {}))]
                   {: id : module}))
  (table.sort modules
              (fn [a b]
                (local left (or a.module.priority 0))
                (local right (or b.module.priority 0))
                (if (= left right) (< a.id b.id) (< left right))))
  (local applications [])
  (each [_ entry (ipairs modules)]
    (local module entry.module)
    (assert (not (and module.source module.build))
            "module must choose source or build")
    (when module.source
      (assert (= (type module.source) :string) "module source must be a name"))
    (local build
           (if module.source (require module.source)
               (assert module.build
                       "module requires source or a pure constructor")))
    (local definitions
           (if (= (type build) :function)
               (build {:config (or spec.config {})})
               (assert build.definitions "module requires definitions")))
    (assert (= (type definitions) :table)
            (.. "module returned no definitions: " entry.id))
    (local events {})
    (each [id event (pairs (or definitions.events {}))]
      (if (= event misa.delete)
          (tset events id misa.delete)
          (let [ordered (collect [key value (pairs event)] key value)]
            (tset ordered :priority
                  (+ (or module.priority 0) (or event.priority 0)))
            (tset events id ordered))))
    (local owned (collect [key value (pairs definitions)] key value))
    (tset owned :events events)
    (table.insert applications {:definitions owned}))
  (table.insert applications
                {:config (or spec.config {})
                 :definitions (or spec.definitions {})})
  (local composed (misa.compose applications))
  {:config composed.config :definitions composed.definitions})

{: application :default (require :misa.default)}
