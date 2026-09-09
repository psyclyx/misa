(local standard (require :misa.standard))

(standard.application
  {:config {"models" {"default" "fake/default"} "providers" {"fake" {"responses" [{"stream" [{"type" "tool_call" "index" 0 "id" "call-1" "name" "echo" "arguments_json_delta" "{\"value\":\"from "} {"type" "tool_call" "index" 0 "arguments_json_delta" "tool\"}"}]} "after tool"]}}}
   :modules {
    "module-1" {:priority 0 :build (require "stream")}
    "module-2" {:priority 1000 :build (require "keybindings")}
    "module-3" {:priority 2000 :build (require "values")}
    "module-4" {:priority 3000 :build (require "json")}
    "module-5" {:priority 4000 :build (require "provider.fake")}
    "module-6" {:priority 5000 :build (require "themes")}
    "module-7" {:priority 6000 :build (require "theme.default")}
    "module-8" {:priority 7000 :build (require "components")}
    "module-9" {:priority 8000 :build (require "layout")}
    "module-10" {:priority 9000 :build (require "markdown")}
    "module-11" {:priority 10000 :build (require "component.markdown")}
    "module-12" {:priority 11000 :build (require "component.content")}
    "module-13" {:priority 12000 :build (require "component.truncation")}
    "module-14" {:priority 13000 :build (require "tool.presentations")}
    "module-15" {:priority 14000 :build (require "component.tool")}
    "module-16" {:priority 15000 :build (require "component.group")}
    "module-17" {:priority 16000 :build (require "component.message")}
    "module-18" {:priority 17000 :build (require "component.editor")}
    "module-19" {:priority 18000 :build (require "component.picker")}
    "module-20" {:priority 19000 :build (require "component.status")}
    "module-21" {:priority 20000 :build (require "component.chrome")}
    "module-22" {:priority 21000 :build (require "messages")}
    "module-23" {:priority 22000 :build (require "models")}
    "module-24" {:priority 23000 :build (require "agent")}
    "module-25" {:priority 24000 :build (require "commands")}
    "module-26" {:priority 25000 :build (require "choices")}
    "module-27" {:priority 26000 :build (require "editor")}
    "module-28" {:priority 27000 :build (require "ui")}
    "module-29" {:priority 28000 :build ((. (require :fennel) :dofile) "@ROOT@/tests/integration/fixtures/tool.fnl")}}})
