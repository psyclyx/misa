(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "misa.text.markdown")}
    "module-2" {:priority 1000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/markdown.fnl")}}})
