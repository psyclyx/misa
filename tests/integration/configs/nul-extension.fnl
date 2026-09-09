(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build ((. (require :fennel) :dofile) "bad\000.fnl")}}})
