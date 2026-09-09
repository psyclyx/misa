(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "misa.agent.stream")}
    "module-2" {:priority 1000 :build (require "misa.providers.unknown")}}})
