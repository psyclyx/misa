(local fennel (require :fennel))
(local standard (require :misa.standard))

(fn application [fixture settings directory]
  "Compose the standard UI with deterministic benchmark inputs."
  (local spec (misa.snapshot standard.default))
  (set spec.config.benchmark settings)
  (set spec.config.models.default
       (if (= fixture :picker-key-repeat) :bench/model-00001 :bench/model))
  (each [_ name (ipairs [:history :themes :components :preferences])]
    (tset spec.config name
          (misa.patch (or (. spec.config name) {}) {:persist false})))
  (each [id module (pairs spec.modules)]
    (if (id:match "^misa%.providers%.") (tset spec.modules id nil)
        (and directory (not= directory :extensions))
        (do
          (local path
                 (assert (fennel.search-module id
                                               (.. directory "/?.fnl;"
                                                   directory "/?/init.fnl"))
                         (.. "missing benchmark module: " id)))
          (set module.source nil)
          (set module.build (fennel.dofile path)))))
  (tset spec.modules :fixture
        {:priority -1000
         :build (fennel.dofile (.. "benchmarks/" fixture ".fnl"))})
  (standard.application spec))

application
