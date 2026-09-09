(local standard (require :misa.standard))

(standard.application
  {:config {"messages" {"verbose" true}}
   :modules {
    "module-1" {:priority 0 :build (require "values")}
    "module-2" {:priority 1000 :build (require "themes")}
    "module-3" {:priority 2000 :build (require "theme.default")}
    "module-4" {:priority 3000 :build (require "components")}
    "module-5" {:priority 4000 :build (require "layout")}
    "module-6" {:priority 5000 :build (require "markdown")}
    "module-7" {:priority 6000 :build (require "component.markdown")}
    "module-8" {:priority 7000 :build (require "component.content")}
    "module-9" {:priority 8000 :build (require "component.truncation")}
    "module-10" {:priority 9000 :build (require "tool.presentations")}
    "module-11" {:priority 10000 :build (require "component.tool")}
    "module-12" {:priority 11000 :build (require "component.group")}
    "module-13" {:priority 12000 :build (require "component.message")}
    "module-14" {:priority 13000 :build (require "costs")}
    "module-15" {:priority 14000 :build (require "messages")}
    "module-16" {:priority 15000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/response-metadata.fnl")}}})
