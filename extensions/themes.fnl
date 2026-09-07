;; Semantic theme data and style composition. Components name roles; this service

;; alone resolves those names to the closed native style record.

{:setup (fn [context]
          (local setup-fx [])
          (local entries {})
          (var config (or (and (= (type context.config) :table)
                               context.config.themes)
                          nil))
          (set config (or (and (= (type config) :table) config) {}))
          (local configured (or (and (= (type config.default) :string)
                                     config.default)
                                :default))
          (local ansi {:black true
                       :blue true
                       :bright_black true
                       :bright_blue true
                       :bright_cyan true
                       :bright_green true
                       :bright_magenta true
                       :bright_red true
                       :bright_white true
                       :bright_yellow true
                       :cyan true
                       :green true
                       :magenta true
                       :red true
                       :white true
                       :yellow true})
          (local attributes {:bold true
                             :dim true
                             :italic true
                             :strikethrough true
                             :underline true})
          ;; Closed contract emitted by the bundled component/Markdown suite. Themes
          ;; may override any role; omitted roles are deliberately composed from
          ;; `plain` plus a presentation-neutral semantic modifier.
          (local standard-tokens
                 [:plain
                  :dim
                  :bold
                  :italic
                  :strikethrough
                  :underline
                  :accent
                  :hover
                  :code
                  :link
                  :quote
                  :markdown.heading.1
                  :markdown.heading.2
                  :markdown.heading.3
                  :markdown.heading.4
                  :markdown.heading.5
                  :markdown.heading.6
                  :markdown.list.marker
                  :markdown.rule
                  :markdown.table.border
                  :markdown.table.header
                  :markdown.code.label
                  :markdown.code.border
                  :syntax.comment
                  :syntax.string
                  :syntax.number
                  :syntax.keyword
                  :syntax.type
                  :syntax.function
                  :syntax.constant
                  :syntax.variable
                  :syntax.property
                  :syntax.tag
                  :syntax.attribute
                  :syntax.operator
                  :syntax.punctuation
                  :syntax.escape
                  :syntax.embedded
                  :surface.user
                  :surface.assistant
                  :surface.thinking
                  :surface.tool
                  :surface.error
                  :surface.harness
                  :surface.dialog
                  :editor.normal
                  :editor.visual
                  :selection
                  :user
                  :assistant
                  :thinking
                  :tool
                  :pending
                  :error
                  :tool.pending
                  :tool.success
                  :tool.error
                  :tool.cancelled
                  :rail.user
                  :rail.assistant
                  :rail.thinking
                  :rail.tool
                  :rail.error
                  :rail.harness
                  :label
                  :value
                  :keybinding
                  :choice.prompt
                  :choice.query
                  :choice.hint
                  :choice.view
                  :choice.view.active
                  :choice.row
                  :choice.row.active
                  :choice.row.selected
                  :choice.empty
                  :choice.preview
                  :dialog.title
                  :dialog.message
                  :dialog.label
                  :dialog.value
                  :dialog.code
                  :dialog.progress
                  :dialog.input
                  :dialog.hint])
          (local dim-fallback
                 {:choice.empty true
                  :choice.hint true
                  :choice.preview true
                  :dialog.hint true
                  :dialog.label true
                  :dialog.progress true
                  :dim true
                  :label true
                  :markdown.code.border true
                  :markdown.rule true
                  :markdown.table.border true
                  :pending true
                  :quote true
                  :rail.harness true
                  :syntax.comment true
                  :syntax.punctuation true
                  :thinking true
                  :tool.cancelled true
                  :tool.pending true})
          (local bold-fallback
                 {:bold true
                  :choice.row.selected true
                  :choice.view true
                  :choice.view.active true
                  :dialog.code true
                  :dialog.title true
                  :error true
                  :markdown.code.label true
                  :markdown.heading.1 true
                  :markdown.heading.2 true
                  :markdown.heading.3 true
                  :markdown.list.marker true
                  :markdown.table.header true
                  :rail.error true
                  :syntax.escape true
                  :syntax.keyword true
                  :syntax.operator true
                  :tool.error true})

          (fn rgb [value]
            (if (not= (type value) :table) nil
                (do
                  (var count 0)
                  (each [key (pairs value)]
                    (assert (or (or (= key :r) (= key :g)) (= key :b))
                            (.. "unknown RGB field: " (tostring key)))
                    (set count (+ count 1)))
                  (assert (= count 3) "RGB color requires r, g, and b")
                  (local result {})
                  (each [_ key (ipairs [:r :g :b])]
                    (local channel (. value key))
                    (assert (and (and (and (= (type channel) :number)
                                           (= (% channel 1) 0))
                                      (>= channel 0))
                                 (<= channel 255))
                            "RGB channels must be bytes")
                    (tset result key channel))
                  result)))

          (fn terminal-color [value]
            (if (and (= (type value) :string) (value:match "^#%x%x%x%x%x%x$"))
                {:b (tonumber (value:sub 6 7) 16)
                 :g (tonumber (value:sub 4 5) 16)
                 :r (tonumber (value:sub 2 3) 16)}
                (if (= (type value) :string)
                    (do
                      (assert (or (= value :default) (. ansi value))
                              (.. "unknown terminal color: " value))
                      value)
                    (assert (rgb value)
                            "foreground must be an ANSI color, default, or RGB record"))))

          (fn copy-color [value]
            (or (and (= (type value) :table) {:b value.b :g value.g :r value.r})
                value))

          (fn normalize-style [value palette name]
            (assert (= (type value) :table)
                    (.. "theme style must be a record: " name))
            (local result {})
            (each [key field (pairs value)]
              (if (or (= key :foreground) (= key :background))
                  (if (and (= (type field) :string)
                           (not= (. palette field) nil))
                      (tset result key (copy-color (. palette field)))
                      (tset result key (terminal-color field)))
                  (do
                    (assert (. attributes key)
                            (.. "unknown style field: " (tostring key)))
                    (assert (= (type field) :boolean)
                            (.. "style attribute must be boolean: " key))
                    (tset result key field))))
            (assert (not= (next result) nil)
                    (.. "theme style must not be empty: " name))
            result)

          (fn normalize-theme [theme]
            (assert (and (= (type theme.palette) :table)
                         (= (type theme.styles) :table))
                    "theme requires palette and styles records")
            (local palette {})
            (each [name color (pairs theme.palette)]
              (assert (and (= (type name) :string) (not= name ""))
                      "palette names must be nonempty strings")
              (tset palette name (terminal-color color)))
            ;; Overrides are independent of component implementation and theme choice.
            ;; Palette overrides resolve before token composition; style overrides patch
            ;; individual attributes instead of replacing an entire visual vocabulary.
            (assert (or (= config.palette nil) (= (type config.palette) :table))
                    "theme palette overrides must be a record")
            (each [name color (pairs (or config.palette {}))]
              (assert (and (= (type name) :string) (not= name ""))
                      "palette names must be nonempty strings")
              (tset palette name (terminal-color color)))
            (local styles {})
            (each [name style (pairs theme.styles)]
              (assert (and (= (type name) :string) (not= name ""))
                      "style token must be nonempty")
              (tset styles name (normalize-style style palette name)))
            (assert (or (= config.styles nil) (= (type config.styles) :table))
                    "theme style overrides must be a record")
            (each [name style (pairs (or config.styles {}))]
              (assert (and (= (type name) :string) (not= name ""))
                      "style token must be nonempty")
              (local target (or (. styles name) {}))
              (tset styles name target)
              (each [key value (pairs (normalize-style style palette name))]
                (tset target key value)))
            (assert styles.plain "theme is missing foundation token: plain")
            (each [_ required (ipairs standard-tokens)]
              (when (not (. styles required))
                (local fallback {})
                (each [key value (pairs styles.plain)]
                  (tset fallback key (copy-color value)))
                (if (or (or (= required :italic) (= required :syntax.embedded))
                        (= required :markdown.heading.4))
                    (set fallback.italic true)
                    (= required :strikethrough)
                    (set fallback.strikethrough true)
                    (or (or (or (= required :underline) (= required :link))
                            (= required :syntax.variable))
                        (= required :markdown.heading.5))
                    (set fallback.underline true)
                    (. bold-fallback required)
                    (set fallback.bold true)
                    (. dim-fallback required)
                    (set fallback.dim true))
                (tset styles required fallback)))
            {: palette : styles})

          (fn merge [target source]
            (each [key value (pairs source)]
              (tset target key (copy-color value)))
            nil)

          (table.insert setup-fx
                        {:type :register/setup-effect
                         :name :register/theme
                         :handler (fn [effect]
                                    (let [id effect.id
                                          theme effect.value]
                                      (assert (and (and (= (type id) :string)
                                                        (not= id ""))
                                                   (= (type theme) :table))
                                              "invalid theme")
                                      (assert (= (. entries id) nil)
                                              (.. "duplicate theme: " id))
                                      (tset entries id (normalize-theme theme))
                                      nil))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :theme
                         :value (fn [db]
                                  (assert (and (= (type db) :table)
                                               (= (type db.themes) :table))
                                          "theme resolution requires initialized db")
                                  (assert (. entries db.themes.active)
                                          (.. "unknown theme: "
                                              (tostring db.themes.active))))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :theme_style
                         :value (fn [db tokens]
                                  (when (= (type tokens) :string)
                                    (set-forcibly! tokens [tokens]))
                                  (assert (and (= (type tokens) :table)
                                               (> (length tokens) 0))
                                          "span style requires one or more semantic tokens")
                                  (local (styles result)
                                         (values (. (misa.theme db) :styles) {}))
                                  (each [_ name (ipairs tokens)]
                                    (assert (and (= (type name) :string)
                                                 (not= name ""))
                                            "semantic style tokens must be nonempty strings")
                                    (merge result
                                           (assert (. styles name)
                                                   (.. "theme has no style token: "
                                                       name))))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :swap_theme
                         :value (fn [db id]
                                  (assert (. entries id)
                                          (.. "unknown theme: " (tostring id)))
                                  (misa.patch db {:themes {:active id}}))})
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (when (= tx.event.type :app/start)
                                             (when (not tx.db.themes)
                                               (set tx.db (misa.patch tx.db
                                                                      {:themes {:active configured}}))))
                                           tx)
                                 :id :themes/initialize}})
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn [db]
                                    (if (= config.persist false) nil
                                        {:fx [{:completion :themes/loaded
                                               :namespace :ui.theme
                                               :type :state/load}]}))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :themes/loaded
                         :handler (fn [db event]
                                    (if (or (= event.found false)
                                            (= event.data misa.json_null))
                                        nil
                                        (do
                                          (assert (and (= (type event.data)
                                                          :table)
                                                       (= (type event.data.active)
                                                          :string))
                                                  "invalid persisted theme")
                                          (when (. entries event.data.active)
                                            {:patch {:themes {:active event.data.active}}}))))})
          (table.insert setup-fx
                        {:type :register/event
                         :name :themes/swap
                         :handler (fn [db event]
                                    (local next (misa.swap_theme db event.theme))
                                    (local fx {})
                                    (when (not= config.persist false)
                                      (tset fx (+ (length fx) 1)
                                            {:data next.themes
                                             :namespace :ui.theme
                                             :type :state/save}))
                                    (tset fx (+ (length fx) 1)
                                          {:event {:type :ui/redraw}
                                           :type :dispatch})
                                    {:patch {:themes (misa.replace next.themes)} : fx})})
          {:fx setup-fx})}
