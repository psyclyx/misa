(local standard (require :misa.standard))

(standard.application
  {:config {"models" {"default" "anthropic/claude-sonnet-5"}}
   :modules {
    "module-1" {:priority 0 :build (require "misa.agent.stream")}
    "module-2" {:priority 1000 :build (require "misa.keybindings")}
    "module-3" {:priority 2000 :build (require "misa.ui.values")}
    "module-4" {:priority 3000 :build (require "misa.protocols.anthropic")}
    "module-5" {:priority 4000 :build (require "misa.providers.anthropic")}
    "module-6" {:priority 5000 :build (require "misa.providers.kimi")}
    "module-7" {:priority 6000 :build (require "misa.protocols.openai")}
    "module-8" {:priority 7000 :build (require "misa.providers.openai")}
    "module-9" {:priority 8000 :build (require "misa.providers.openrouter")}
    "module-10" {:priority 9000 :build (require "misa.providers.openai-codex")}
    "module-11" {:priority 10000 :build (require "misa.providers.claude")}
    "module-12" {:priority 11000 :build (require "misa.ui.themes")}
    "module-13" {:priority 12000 :build (require "misa.ui.themes.default")}
    "module-14" {:priority 13000 :build (require "misa.ui.components")}
    "module-15" {:priority 14000 :build (require "misa.ui.layout")}
    "module-16" {:priority 15000 :build (require "misa.markdown")}
    "module-17" {:priority 16000 :build (require "misa.markdown.render")}
    "module-18" {:priority 17000 :build (require "misa.ui.components.content")}
    "module-19" {:priority 18000 :build (require "misa.ui.components.truncation")}
    "module-20" {:priority 19000 :build (require "misa.transcript.tools")}
    "module-21" {:priority 20000 :build (require "misa.transcript.tools.render")}
    "module-22" {:priority 21000 :build (require "misa.ui.components.group")}
    "module-23" {:priority 22000 :build (require "misa.transcript.render")}
    "module-24" {:priority 23000 :build (require "misa.editor.render")}
    "module-25" {:priority 24000 :build (require "misa.choices.picker.render")}
    "module-26" {:priority 25000 :build (require "misa.ui.status.render")}
    "module-27" {:priority 26000 :build (require "misa.ui.chrome")}
    "module-28" {:priority 27000 :build (require "misa.transcript")}
    "module-29" {:priority 28000 :build (require "misa.models")}
    "module-30" {:priority 29000 :build (require "misa.agent")}
    "module-31" {:priority 30000 :build (require "misa.commands")}
    "module-32" {:priority 31000 :build (require "misa.choices")}
    "module-33" {:priority 32000 :build (require "misa.editor")}
    "module-34" {:priority 33000 :build (require "misa.ui")}
    :misa.transcript.groups {:source :misa.transcript.groups}}})
