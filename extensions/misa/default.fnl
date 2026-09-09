{:config {:models {:default "claude/claude-sonnet-5"}
          :choices {:overlay {:padding 2
                              :preferred_height 14
                              :min_height 4
                              :max_height 18}
                    :purposes {:command-completion ["all"]
                               :command ["browse" "favorites"]
                               :models ["browse" "favorites"]
                               :auth ["all"]
                               :actions ["browse" "favorites"]}}
          :keybindings {:global {:toggle_verbose ["alt+t"]
                                 :transcript_up ["page_up" "alt+k"]
                                 :transcript_down ["page_down" "alt+j"]
                                 :cycle_effort ["alt+f"]
                                 :open_omnipicker ["alt+/"]}
                        :choices {:accept ["enter"]
                                  :cancel ["escape" "ctrl_c" "ctrl_d" "eof"]
                                  :previous ["arrow_up"]
                                  :next ["arrow_down"]
                                  :cycle ["arrow_right"]
                                  :cycle_previous ["arrow_left" "alt+p"]
                                  :favorite ["alt+v"]
                                  :replace_view ["alt+/"]
                                  :open_overlay []
                                  :complete ["tab"]}}
          :components {:roles {:root.header "default.root.header"
                               :editor.input "default.editor.input"
                               :editor.completions "default.editor.completions"
                               :status.indicators "default.status.indicators"
                               :picker "default.picker"
                               :dialog "default.dialog"
                               :transcript.user "default.transcript.user"
                               :transcript.assistant "default.transcript.assistant"
                               :transcript.thinking "default.transcript.thinking"
                               :transcript.thinking_collapsed "default.transcript.thinking_collapsed"
                               :transcript.tool_call "default.transcript.tool_call"
                               :transcript.tool_result "default.transcript.tool_result"
                               :transcript.harness "default.transcript.harness"
                               :selection "default.selection"
                               :data "default.data"}}
          :themes {:default "default" :persist true}
          :animations {:default "default" :enabled true :interval_ms 160}
          :messages {:verbose false
                     :markdown true
                     :max_string 4000
                     :max_items 64
                     :max_depth 8}
          :status {:indicators [{:id "activity"
                                 :representation "icon"
                                 :priority 100}
                                {:id "cost"
                                 :representation "label"
                                 :priority 85}
                                {:id "model"
                                 :representation "value"
                                 :hotkey true
                                 :priority 90}
                                {:id "effort"
                                 :representation "label"
                                 :hotkey true
                                 :priority 70}
                                {:id "session"
                                 :representation "icon"
                                 :priority 40}
                                {:id "context"
                                 :representation "label"
                                 :priority 80}
                                {:id "plan"
                                 :representation "label"
                                 :priority 60}
                                {:id "transcript-detail"
                                 :representation "label"
                                 :hotkey true
                                 :priority 20}]}
          :ui {:plain_prompt true}
          :editing {:mode "vim"}
          :history {:persist true :max_entries 500}}
 :modules {:values {:priority 0 :source :values}
           :json {:priority 1000 :source :json}
           :stream {:priority 2000 :source :stream}
           :protocol.anthropic {:priority 3000 :source :protocol.anthropic}
           :provider.anthropic {:priority 4000 :source :provider.anthropic}
           :provider.kimi {:priority 5000 :source :provider.kimi}
           :protocol.openai {:priority 6000 :source :protocol.openai}
           :provider.openai {:priority 7000 :source :provider.openai}
           :provider.openrouter {:priority 8000 :source :provider.openrouter}
           :provider.openai-codex {:priority 9000
                                   :source :provider.openai-codex}
           :provider.claude {:priority 10000 :source :provider.claude}
           :auth {:priority 11000 :source :auth}
           :tool.files {:priority 12000 :source :tool.files}
           :tool.shell {:priority 13000 :source :tool.shell}
           :fuzzy {:priority 14000 :source :fuzzy}
           :keybindings {:priority 15000 :source :keybindings}
           :actions {:priority 16000 :source :actions}
           :links {:priority 17000 :source :links}
           :clipboard {:priority 18000 :source :clipboard}
           :dialogs {:priority 19000 :source :dialogs}
           :commands {:priority 20000 :source :commands}
           :choices {:priority 21000 :source :choices}
           :preferences {:priority 22000 :source :preferences}
           :themes {:priority 23000 :source :themes}
           :theme.default {:priority 24000 :source :theme.default}
           :animations {:priority 25000 :source :animations}
           :animation.default {:priority 26000 :source :animation.default}
           :components {:priority 27000 :source :components}
           :layout {:priority 28000 :source :layout}
           :choices.preview {:priority 29000 :source :choices.preview}
           :choices.layout {:priority 30000 :source :choices.layout}
           :markdown {:priority 31000 :source :markdown}
           :syntax {:priority 32000 :source :syntax}
           :selection.document {:priority 33000 :source :selection.document}
           :selection {:priority 34000 :source :selection}
           :component.markdown {:priority 35000 :source :component.markdown}
           :component.image {:priority 36000 :source :component.image}
           :indicators {:priority 37000 :source :indicators}
           :component.content {:priority 38000 :source :component.content}
           :component.truncation {:priority 39000
                                  :source :component.truncation}
           :tool.presentations {:priority 40000 :source :tool.presentations}
           :component.tool {:priority 41000 :source :component.tool}
           :component.group {:priority 42000 :source :component.group}
           :component.message {:priority 43000 :source :component.message}
           :component.editor {:priority 44000 :source :component.editor}
           :component.picker {:priority 45000 :source :component.picker}
           :component.status {:priority 46000 :source :component.status}
           :component.chrome {:priority 47000 :source :component.chrome}
           :component.buttons {:priority 48000 :source :component.buttons}
           :component.data {:priority 49000 :source :component.data}
           :component.dialog {:priority 50000 :source :component.dialog}
           :component.selection {:priority 51000 :source :component.selection}
           :dialogs.view {:priority 52000 :source :dialogs.view}
           :messages {:priority 53000 :source :messages}
           :status {:priority 54000 :source :status}
           :usage {:priority 55000 :source :usage}
           :picker {:priority 56000 :source :picker}
           :picker.view {:priority 57000 :source :picker.view}
           :models {:priority 58000 :source :models}
           :costs {:priority 59000 :source :costs}
           :omnipicker {:priority 60000 :source :omnipicker}
           :request-options {:priority 61000 :source :request-options}
           :effort {:priority 62000 :source :effort}
           :agent {:priority 63000 :source :agent}
           :tool.summary {:priority 64000 :source :tool.summary}
           :queue {:priority 65000 :source :queue}
           :queue.view {:priority 66000 :source :queue.view}
           :editor {:priority 67000 :source :editor}
           :images {:priority 68000 :source :images}
           :attachments {:priority 69000 :source :attachments}
           :history {:priority 70000 :source :history}
           :editing {:priority 71000 :source :editing}
           :ui {:priority 72000 :source :ui}}}
