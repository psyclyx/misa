(local chrome (require :misa.ui.chrome))
(local buttons (require :misa.ui.components.buttons))
(local content (require :misa.ui.components.content))
(local data (require :misa.ui.components.data))
(local group (require :misa.ui.components.group))
(local truncation (require :misa.ui.components.truncation))
(local messages (require :misa.transcript.render))
(local groups (require :misa.transcript.groups))
(local tools (require :misa.transcript.tools.render))
(local status (require :misa.ui.status.render))

{:services {:components.buttons buttons.components-buttons}
 :value-renderers {:activity status.activity}
 :components {:default.root.header {:render chrome.default-root-header-render}
              :default.content.text {:render content.text}
              :default.content.fields {:render content.fields}
              :default.content.code {:render content.code}
              :default.content.lines {:render content.code}
              :default.content.diff {:render (fn [model context]
                                               (content.code model context
                                                             :diff))}
              :default.content.truncated {:render truncation.render}
              :default.data {:render data.render}
              :default.group.boundary {:render group.boundary}
              :default.status.indicators {:render status.render-indicators}
              :default.transcript.group_header {:render groups.header
                                                :compose true}
              :default.transcript.group_footer {:render groups.footer
                                                :compose true}
              :default.transcript.tool_call {:render tools.render
                                             :compose true}
              :default.transcript.tool_result {:render tools.render
                                               :compose true}
              :default.transcript.user {:render (fn [model context previous]
                                                  (messages.render-message :user
                                                                           true
                                                                           model
                                                                           context
                                                                           previous))}
              :default.transcript.assistant {:render (fn [model
                                                          context
                                                          previous]
                                                       (messages.render-message :assistant
                                                                                false
                                                                                model
                                                                                context
                                                                                previous))}
              :default.transcript.thinking {:render (fn [model
                                                         context
                                                         previous]
                                                      (messages.render-message :thinking
                                                                               false
                                                                               model
                                                                               context
                                                                               previous))}
              :default.transcript.thinking_collapsed {:render messages.collapsed
                                                      :compose true}
              :default.transcript.harness {:render messages.harness}}
 :requirements {:component.buttons [:keybindings.reference]
                :component.content [:layout :markdown.view]
                :component.data [:layout :components.buttons :values.render]
                :component.group [:layout :values.render]
                :component.message [:layout :markdown :markdown.view]
                :component.tool [:layout :values.render :tools.presentation]
                :component.status [:values.render]}}
