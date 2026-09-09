(local standard (require :misa.standard))

(standard.application
  {:config {"models" {"default" "private/model"}}
   :modules {
    "module-1" {:priority 0 :build (require "keybindings")}
    "module-2" {:priority 1000 :build (require "values")}
    "module-3" {:priority 2000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/unavailable-models.fnl")}
    "module-4" {:priority 3000 :build (require "auth")}
    "module-5" {:priority 4000 :build (require "themes")}
    "module-6" {:priority 5000 :build (require "theme.default")}
    "module-7" {:priority 6000 :build (require "components")}
    "module-8" {:priority 7000 :build (require "layout")}
    "module-9" {:priority 8000 :build (require "markdown")}
    "module-10" {:priority 9000 :build (require "component.markdown")}
    "module-11" {:priority 10000 :build (require "component.content")}
    "module-12" {:priority 11000 :build (require "component.truncation")}
    "module-13" {:priority 12000 :build (require "tool.presentations")}
    "module-14" {:priority 13000 :build (require "component.tool")}
    "module-15" {:priority 14000 :build (require "component.group")}
    "module-16" {:priority 15000 :build (require "component.message")}
    "module-17" {:priority 16000 :build (require "component.editor")}
    "module-18" {:priority 17000 :build (require "component.picker")}
    "module-19" {:priority 18000 :build (require "component.status")}
    "module-20" {:priority 19000 :build (require "component.chrome")}
    "module-21" {:priority 20000 :build (require "messages")}
    "module-22" {:priority 21000 :build (require "models")}
    "module-23" {:priority 22000 :build (require "agent")}
    "module-24" {:priority 23000 :build (require "commands")}
    "module-25" {:priority 24000 :build (require "choices")}
    "module-26" {:priority 25000 :build (require "editor")}
    "module-27" {:priority 26000 :build (require "ui")}}})
