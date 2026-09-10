{:models {:default :claude/claude-sonnet-5}
 :choices {:overlay {:padding 2
                     :preferred_height 14
                     :min_height 4
                     :max_height 18}
           :purposes {:command-completion [:all]
                      :command [:browse :favorites]
                      :models [:browse :favorites]
                      :auth [:all]
                      :actions [:browse :favorites]}}
 :compaction {:enabled true
              :max_input_bytes 400000
              :max_summary_bytes 64000
              :min_messages 6
              :reserve_tokens 16000
              :role :summarizer
              :threshold 0.8}
 :components {:roles {:root.header :default.root.header
                      :editor.input :default.editor.input
                      :editor.completions :default.editor.completions
                      :status.indicators :default.status.indicators
                      :picker :default.picker
                      :dialog :default.dialog
                      :transcript.user :default.transcript.user
                      :transcript.assistant :default.transcript.assistant
                      :transcript.thinking :default.transcript.thinking
                      :transcript.thinking_collapsed :default.transcript.thinking_collapsed
                      :transcript.tool_call :default.transcript.tool_call
                      :transcript.tool_result :default.transcript.tool_result
                      :transcript.harness :default.transcript.harness
                      :selection :default.selection
                      :data :default.data}}
 :themes {:default :default :persist true}
 :animations {:default :default :enabled true :interval_ms 160}
 :messages {:verbose false
            :markdown true
            :max_string 4000
            :max_items 64
            :max_depth 8}
 :status {:indicators [{:id :activity :representation :icon :priority 100}
                       {:id :cost :representation :label :priority 85}
                       {:id :model
                        :representation :value
                        :hotkey true
                        :priority 90}
                       {:id :effort
                        :representation :label
                        :hotkey true
                        :priority 70}
                       {:id :session :representation :icon :priority 40}
                       {:id :context :representation :label :priority 80}
                       {:id :plan :representation :label :priority 60}
                       {:id :transcript-detail
                        :representation :label
                        :hotkey true
                        :priority 20}]}
 :ui {:plain_prompt true}
 :editing {:mode :vim}
 :history {:persist true :max_entries 500}}
