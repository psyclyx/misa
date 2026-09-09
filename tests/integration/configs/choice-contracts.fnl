(local standard (require :misa.standard))

(standard.application
  {:config {"choices" {"purposes" {"models" ["frecency" "all"]}} "keybindings" {"choices" {"open_overlay" ["alt+x"] "option_1_1" ["alt+z"]}}}
   :modules {
    "module-1" {:priority 0 :build (require "values")}
    "module-2" {:priority 1000 :build (require "fuzzy")}
    "module-3" {:priority 2000 :build (require "keybindings")}
    "module-4" {:priority 3000 :build (require "layout")}
    "module-5" {:priority 4000 :build (require "commands")}
    "module-6" {:priority 5000 :build (require "choices")}
    "module-7" {:priority 6000 :build (require "choices.preview")}
    "module-8" {:priority 7000 :build (require "choices.layout")}
    "module-9" {:priority 8000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/choice-contracts.fnl")}}})
