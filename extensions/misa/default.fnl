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
 :modules {:misa.agent {:priority 63000 :source :misa.agent}
           :misa.agent.costs {:priority 59000 :source :misa.agent.costs}
           :misa.agent.effort {:priority 62000 :source :misa.agent.effort}
           :misa.agent.models {:priority 58000 :source :misa.agent.models}
           :misa.agent.request-options {:priority 61000
                                        :source :misa.agent.request-options}
           :misa.choices {:priority 21000 :source :misa.choices}
           :misa.choices.layout {:priority 30000 :source :misa.choices.layout}
           :misa.choices.picker {:priority 56000 :source :misa.choices.picker}
           :misa.choices.picker.view {:priority 57000
                                      :source :misa.choices.picker.view}
           :misa.choices.preferences {:priority 22000
                                      :source :misa.choices.preferences}
           :misa.choices.preview {:priority 29000
                                  :source :misa.choices.preview}
           :misa.commands {:priority 20000 :source :misa.commands}
           :misa.commands.actions {:priority 16000
                                   :source :misa.commands.actions}
           :misa.commands.keybindings {:priority 15000
                                       :source :misa.commands.keybindings}
           :misa.commands.palette {:priority 60000
                                   :source :misa.commands.palette}
           :misa.dialogs {:priority 19000 :source :misa.dialogs}
           :misa.dialogs.view {:priority 52000 :source :misa.dialogs.view}
           :misa.editor {:priority 67000 :source :misa.editor}
           :misa.editor.attachments {:priority 69000
                                     :source :misa.editor.attachments}
           :misa.editor.editing {:priority 71000 :source :misa.editor.editing}
           :misa.editor.history {:priority 70000 :source :misa.editor.history}
           :misa.editor.images {:priority 68000 :source :misa.editor.images}
           :misa.editor.queue {:priority 65000 :source :misa.editor.queue}
           :misa.editor.queue.view {:priority 66000
                                    :source :misa.editor.queue.view}
           :misa.json {:priority 1000 :source :misa.json}
           :misa.protocols.anthropic {:priority 3000
                                      :source :misa.protocols.anthropic}
           :misa.protocols.openai {:priority 6000
                                   :source :misa.protocols.openai}
           :misa.protocols.stream {:priority 2000
                                   :source :misa.protocols.stream}
           :misa.providers.anthropic {:priority 4000
                                      :source :misa.providers.anthropic}
           :misa.providers.auth {:priority 11000 :source :misa.providers.auth}
           :misa.providers.claude {:priority 10000
                                   :source :misa.providers.claude}
           :misa.providers.kimi {:priority 5000 :source :misa.providers.kimi}
           :misa.providers.openai {:priority 7000
                                   :source :misa.providers.openai}
           :misa.providers.openai-codex {:priority 9000
                                         :source :misa.providers.openai-codex}
           :misa.providers.openrouter {:priority 8000
                                       :source :misa.providers.openrouter}
           :misa.selection {:priority 34000 :source :misa.selection}
           :misa.selection.document {:priority 33000
                                     :source :misa.selection.document}
           :misa.system.clipboard {:priority 18000
                                   :source :misa.system.clipboard}
           :misa.system.links {:priority 17000 :source :misa.system.links}
           :misa.text.fuzzy {:priority 14000 :source :misa.text.fuzzy}
           :misa.text.markdown {:priority 31000 :source :misa.text.markdown}
           :misa.text.syntax {:priority 32000 :source :misa.text.syntax}
           :misa.tools.files {:priority 12000 :source :misa.tools.files}
           :misa.tools.shell {:priority 13000 :source :misa.tools.shell}
           :misa.tools.summary {:priority 64000 :source :misa.tools.summary}
           :misa.ui {:priority 72000 :source :misa.ui}
           :misa.ui.animations {:priority 25000 :source :misa.ui.animations}
           :misa.ui.animations.default {:priority 26000
                                        :source :misa.ui.animations.default}
           :misa.ui.components {:priority 27000 :source :misa.ui.components}
           :misa.ui.components.buttons {:priority 48000
                                        :source :misa.ui.components.buttons}
           :misa.ui.components.chrome {:priority 47000
                                       :source :misa.ui.components.chrome}
           :misa.ui.components.content {:priority 38000
                                        :source :misa.ui.components.content}
           :misa.ui.components.data {:priority 49000
                                     :source :misa.ui.components.data}
           :misa.ui.components.dialog {:priority 50000
                                       :source :misa.ui.components.dialog}
           :misa.ui.components.editor {:priority 44000
                                       :source :misa.ui.components.editor}
           :misa.ui.components.group {:priority 42000
                                      :source :misa.ui.components.group}
           :misa.ui.components.image {:priority 36000
                                      :source :misa.ui.components.image}
           :misa.ui.components.markdown {:priority 35000
                                         :source :misa.ui.components.markdown}
           :misa.ui.components.message {:priority 43000
                                        :source :misa.ui.components.message}
           :misa.ui.components.picker {:priority 45000
                                       :source :misa.ui.components.picker}
           :misa.ui.components.selection {:priority 51000
                                          :source :misa.ui.components.selection}
           :misa.ui.components.status {:priority 46000
                                       :source :misa.ui.components.status}
           :misa.ui.components.tool {:priority 41000
                                     :source :misa.ui.components.tool}
           :misa.ui.components.truncation {:priority 39000
                                           :source :misa.ui.components.truncation}
           :misa.ui.layout {:priority 28000 :source :misa.ui.layout}
           :misa.ui.status {:priority 54000 :source :misa.ui.status}
           :misa.ui.status.indicators {:priority 37000
                                       :source :misa.ui.status.indicators}
           :misa.ui.status.usage {:priority 55000
                                  :source :misa.ui.status.usage}
           :misa.ui.themes {:priority 23000 :source :misa.ui.themes}
           :misa.ui.themes.default {:priority 24000
                                    :source :misa.ui.themes.default}
           :misa.ui.tools {:priority 40000 :source :misa.ui.tools}
           :misa.ui.transcript {:priority 53000 :source :misa.ui.transcript}
           :misa.ui.values {:priority 0 :source :misa.ui.values}}}
