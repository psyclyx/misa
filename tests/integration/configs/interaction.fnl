(local standard (require :misa.standard))

(standard.application
  {:config {"components" {"persist" false} "themes" {"persist" false} "preferences" {"persist" false}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.ui.values")}
    "module-2" {:priority 1000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/interaction.fnl")}
    "module-3" {:priority 2000 :build (require "misa.json")}
    "module-4" {:priority 3000 :build (require "misa.text.fuzzy")}
    "module-5" {:priority 4000 :build (require "misa.commands.keybindings")}
    "module-6" {:priority 5000 :build (require "misa.commands.actions")}
    "module-7" {:priority 6000 :build (require "misa.system.clipboard")}
    "module-8" {:priority 7000 :build (require "misa.dialogs")}
    "module-9" {:priority 8000 :build (require "misa.commands")}
    "module-10" {:priority 9000 :build (require "misa.choices")}
    "module-11" {:priority 10000 :build (require "misa.choices.preferences")}
    "module-12" {:priority 11000 :build (require "misa.ui.themes")}
    "module-13" {:priority 12000 :build (require "misa.ui.themes.default")}
    "module-14" {:priority 13000 :build (require "misa.ui.components")}
    "module-15" {:priority 14000 :build (require "misa.ui.layout")}
    "module-16" {:priority 15000 :build (require "misa.choices.preview")}
    "module-17" {:priority 16000 :build (require "misa.choices.layout")}
    "module-18" {:priority 17000 :build (require "misa.text.markdown")}
    "module-19" {:priority 18000 :build (require "misa.selection.document")}
    "module-20" {:priority 19000 :build (require "misa.selection")}
    "module-21" {:priority 20000 :build (require "misa.ui.components.markdown")}
    "module-22" {:priority 21000 :build (require "misa.ui.components.truncation")}
    "module-23" {:priority 22000 :build (require "misa.ui.components.group")}
    "module-24" {:priority 23000 :build (require "misa.ui.components.message")}
    "module-25" {:priority 24000 :build (require "misa.ui.components.editor")}
    "module-26" {:priority 25000 :build (require "misa.ui.components.picker")}
    "module-27" {:priority 26000 :build (require "misa.ui.components.status")}
    "module-28" {:priority 27000 :build (require "misa.ui.components.chrome")}
    "module-29" {:priority 28000 :build (require "misa.ui.components.selection")}
    "module-30" {:priority 29000 :build (require "misa.ui.transcript")}
    "module-31" {:priority 30000 :build (require "misa.choices.picker")}
    "module-32" {:priority 31000 :build (require "misa.choices.picker.view")}
    "module-33" {:priority 32000 :build (require "misa.commands.palette")}
    "module-34" {:priority 33000 :build (require "misa.editor")}
    "module-35" {:priority 34000 :build (require "misa.editor.editing")}
    "module-36" {:priority 35000 :build (require "misa.ui")}}})
