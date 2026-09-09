(local standard (require :misa.standard))

(standard.application
  {:config {}
   :modules {
    "module-1" {:priority 0 :build (require "misa.keybindings")}
    "module-2" {:priority 1000 :build (require "misa.ui.values")}
    "module-3" {:priority 2000 :build (require "misa.ui.themes")}
    "module-4" {:priority 3000 :build (require "misa.ui.themes.default")}
    "module-5" {:priority 4000 :build (require "misa.ui.components")}
    "module-6" {:priority 5000 :build (require "misa.ui.layout")}
    "module-7" {:priority 6000 :build (require "misa.markdown")}
    "module-8" {:priority 7000 :build (require "misa.markdown.render")}
    "module-9" {:priority 8000 :build (require "misa.ui.components.content")}
    "module-10" {:priority 9000 :build (require "misa.ui.components.truncation")}
    "module-11" {:priority 10000 :build (require "misa.transcript.tools")}
    "module-12" {:priority 11000 :build (require "misa.transcript.tools.render")}
    "module-13" {:priority 12000 :build (require "misa.ui.components.group")}
    "module-14" {:priority 13000 :build (require "misa.transcript.render")}
    "module-15" {:priority 14000 :build (require "misa.editor.render")}
    "module-16" {:priority 15000 :build (require "misa.choices.picker.render")}
    "module-17" {:priority 16000 :build (require "misa.ui.status.render")}
    "module-18" {:priority 17000 :build (require "misa.ui.chrome")}
    "module-19" {:priority 18000 :build (require "misa.commands")}
    "module-20" {:priority 19000 :build (require "misa.choices")}
    "module-21" {:priority 20000 :build (require "misa.editor")}
    "module-22" {:priority 21000 :build (require "misa.ui")}
    "module-23" {:priority 22000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/command-completion.fnl")}
    :misa.transcript.groups {:source :misa.transcript.groups}}})
