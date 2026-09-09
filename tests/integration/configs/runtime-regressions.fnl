(local standard (require :misa.standard))

(standard.application
  {:config {"models" {"default" "claude/claude-sonnet-5"} "providers" {"claude" {"executable" "@WORK@/claude"}}}
   :modules {
    "module-1" {:priority 0 :build (require "stream")}
    "module-2" {:priority 1000 :build (require "json")}
    "module-3" {:priority 2000 :build (require "provider.claude")}
    "module-4" {:priority 3000 :build (require "tool.shell")}
    "module-5" {:priority 4000 :build (require "models")}
    "module-6" {:priority 5000 :build (require "agent")}
    "module-7" {:priority 6000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/runtime-regressions-check.fnl")}}})
