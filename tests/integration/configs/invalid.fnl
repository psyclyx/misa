(local standard (require :misa.standard))

(standard.application
  {:config {"models" {"default" "fake/default"} "providers" {"fake" {"responses" [{"stream" [{"type" "tool_call" "id" "invalid" "name" "shell" "arguments" {}}]} "recovered"]}}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.protocols.stream")}
    "module-2" {:priority 1000 :build (require "misa.json")}
    "module-3" {:priority 2000 :build (require "misa.providers.fake")}
    "module-4" {:priority 3000 :build (require "misa.tools.shell")}
    "module-5" {:priority 4000 :build (require "misa.agent.models")}
    "module-6" {:priority 5000 :build (require "misa.agent")}
    "module-7" {:priority 6000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/runtime-regressions-invalid.fnl")}}})
