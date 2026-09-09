(local standard (require :misa.standard))

(standard.application
  {:config {"preferences" {"persist" false}}
   :modules {
    "module-1" {:priority 0 :build (require "values")}
    "module-2" {:priority 1000 :build (require "fuzzy")}
    "module-3" {:priority 2000 :build (require "keybindings")}
    "module-4" {:priority 3000 :build (require "commands")}
    "module-5" {:priority 4000 :build (require "choices")}
    "module-6" {:priority 5000 :build (require "preferences")}
    "module-7" {:priority 6000 :build (require "themes")}
    "module-8" {:priority 7000 :build (require "theme.default")}
    "module-9" {:priority 8000 :build (require "components")}
    "module-10" {:priority 9000 :build (require "layout")}
    "module-11" {:priority 10000 :build (require "choices.preview")}
    "module-12" {:priority 11000 :build (require "choices.layout")}
    "module-13" {:priority 12000 :build (require "component.editor")}
    "module-14" {:priority 13000 :build (require "component.picker")}
    "module-15" {:priority 14000 :build (require "picker")}
    "module-16" {:priority 15000 :build (require "editor")}
    "module-17" {:priority 16000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/command-choice.fnl")}}})
