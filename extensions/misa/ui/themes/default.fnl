(local definitions (require :misa.definitions))

;; Terminal text and the page background remain inherited. Accent colors and
;; raised surfaces are deliberately separate, configurable RGB palette entries.

(fn [context]
  "Build the declarations for theme default."
  (local declarations [])
  (local config (or (. (or context.config {}) :themes) {}))
  (local appearance (or config.appearance :dark))
  (assert (or (= appearance :dark) (= appearance :light))
          "themes.appearance must be dark or light")
  (local palette (or (and (= appearance :light)
                          {:accent "#267C83"
                           :assistant "#527CAF"
                           :assistant_surface "#EEF2F8"
                           :background :default
                           :cancelled "#78828A"
                           :constant "#8866A4"
                           :code_surface "#E2E8EF"
                           :dialog_surface "#EEF2F4"
                           :error "#B24E59"
                           :error_surface "#FAEEF0"
                           :escape "#9C752F"
                           :function "#427AB0"
                           :keyword "#287D89"
                           :muted "#68757F"
                           :number "#986386"
                           :property "#428486"
                           :selection "#CDDFE3"
                           :string "#407B55"
                           :success "#347552"
                           :text :default
                           :thinking "#8967A3"
                           :thinking_surface "#F4EFF7"
                           :tool "#9A7638"
                           :tool_surface "#F7F3E9"
                           :type "#936E31"
                           :user "#448565"
                           :user_surface "#EEF5F0"})
                     {:accent "#81BDB5"
                      :assistant "#8DAFD2"
                      :assistant_surface "#1E252F"
                      :background :default
                      :cancelled "#84919C"
                      :constant "#B4A4DA"
                      :code_surface "#151B23"
                      :dialog_surface "#242B33"
                      :error "#DE9397"
                      :error_surface "#322329"
                      :escape "#D5B783"
                      :function "#96B5DA"
                      :keyword "#86BFC4"
                      :muted "#84919C"
                      :number "#C2A2C9"
                      :property "#9DBFB5"
                      :selection "#3B5260"
                      :string "#A7C799"
                      :success "#97C49E"
                      :text :default
                      :thinking "#B8A1C9"
                      :thinking_surface "#282330"
                      :tool "#C9B07F"
                      :tool_surface "#2B2820"
                      :type "#D0BB86"
                      :user "#93B99A"
                      :user_surface "#1D2824"}))
  (table.insert declarations
                {:catalog :themes
                 :id :default
                 :value {: palette
                         :styles {:accent {:foreground :accent}
                                  :assistant {:foreground :text}
                                  :bold {:bold true}
                                  :choice.empty {:dim true :foreground :text}
                                  :choice.hint {:dim true :foreground :text}
                                  :choice.preview {:dim true :foreground :text}
                                  :choice.prompt {:foreground :accent}
                                  :choice.query {:foreground :text}
                                  :choice.row {:foreground :text}
                                  :choice.row.active {:foreground :accent}
                                  :choice.row.selected {:background :selection
                                                        :bold true}
                                  :choice.view {:bold true}
                                  :choice.view.active {:bold true
                                                       :foreground :accent}
                                  :code {:foreground :accent}
                                  :surface.code {:background :code_surface}
                                  :dialog.code {:bold true :foreground :accent}
                                  :dialog.hint {:dim true :foreground :text}
                                  :dialog.input {:foreground :text}
                                  :dialog.label {:dim true :foreground :text}
                                  :dialog.message {:foreground :text}
                                  :dialog.progress {:dim true
                                                    :foreground :text}
                                  :dialog.title {:bold true
                                                 :foreground :accent}
                                  :dialog.value {:foreground :text}
                                  :dim {:dim true}
                                  :disabled {:dim true
                                             :foreground :muted
                                             :bold false
                                             :underline false}
                                  :editor.normal {:bold true
                                                  :foreground :accent}
                                  :editor.visual {:bold true
                                                  :foreground :accent}
                                  :error {:bold true :foreground :error}
                                  :italic {:italic true}
                                  :keybinding {:dim true :foreground :accent}
                                  :hover {:background :selection}
                                  :label {:dim true :foreground :text}
                                  :link {:foreground :accent :underline true}
                                  :markdown.code.border {:dim true}
                                  :markdown.code.label {:bold true}
                                  :diff.added {:foreground :success}
                                  :diff.removed {:foreground :error}
                                  :markdown.heading.1 {:bold true
                                                       :underline true}
                                  :markdown.heading.2 {:bold true}
                                  :markdown.heading.3 {:bold true :italic true}
                                  :markdown.heading.4 {:italic true}
                                  :markdown.heading.5 {:underline true}
                                  :markdown.heading.6 {:dim true}
                                  :markdown.list.marker {:bold true}
                                  :markdown.rule {:dim true}
                                  :markdown.table.border {:dim true}
                                  :markdown.table.header {:bold true}
                                  :pending {:dim true :foreground :accent}
                                  :plain {:foreground :text}
                                  :quote {:dim true}
                                  :rail.assistant {:foreground :assistant}
                                  :rail.error {:bold true :foreground :error}
                                  :rail.harness {:foreground :muted}
                                  :rail.thinking {:foreground :thinking}
                                  :rail.tool {:foreground :tool}
                                  :rail.user {:foreground :user}
                                  :selection {:background :selection
                                              :underline true}
                                  :strikethrough {:strikethrough true}
                                  :surface.assistant {:background :assistant_surface}
                                  :surface.dialog {:background :dialog_surface}
                                  :surface.error {:background :error_surface}
                                  :surface.harness {:background :default}
                                  :surface.thinking {:background :thinking_surface}
                                  :surface.tool {:background :tool_surface}
                                  :surface.user {:background :user_surface}
                                  :syntax.attribute {:foreground :type}
                                  :syntax.comment {:dim true :italic true}
                                  :syntax.constant {:foreground :constant}
                                  :syntax.embedded {:italic true}
                                  :syntax.escape {:bold true
                                                  :foreground :escape}
                                  :syntax.function {:foreground :function}
                                  :syntax.keyword {:bold true
                                                   :foreground :keyword}
                                  :syntax.number {:foreground :number}
                                  :syntax.operator {:bold true}
                                  :syntax.property {:foreground :property}
                                  :syntax.punctuation {:dim true}
                                  :syntax.string {:foreground :string}
                                  :syntax.tag {:foreground :function}
                                  :syntax.type {:foreground :type}
                                  :syntax.variable {:underline true}
                                  :thinking {:dim true :foreground :text}
                                  :tool {:foreground :text}
                                  :tool.cancelled {:dim true
                                                   :foreground :cancelled}
                                  :tool.error {:bold true :foreground :error}
                                  :tool.pending {:dim true :foreground :accent}
                                  :tool.success {:foreground :success}
                                  :underline {:underline true}
                                  :user {:foreground :text}
                                  :value {:foreground :text}}}})
  (definitions :theme.default declarations {}))
