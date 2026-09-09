(local standard (require :misa.standard))

(standard.application
  {:config {"providers" {"command" {"argv" ["bad\000arg"]}}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.protocols.stream")}
    "module-2" {:priority 1000 :build (require "misa.providers.command")}}})
