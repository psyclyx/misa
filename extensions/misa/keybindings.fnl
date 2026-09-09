(local definitions (require :misa.definitions))

;; Configurable semantic keybindings. Escape-prefixed printable input is exposed
;; as Alt+key without asking feature extensions to parse terminal byte streams.

(fn key-of [event]
  (if (= event.kind :alt_enter)
      :alt+enter
      (if (= event.kind :shift_enter)
          :shift+enter
          (if (= event.kind :key) event.key
              (if (= event.kind :text) event.text
                  (if (and (= event.kind :alt) (= (type event.text) :string))
                      (.. :alt+ (event.text:lower))
                      event.kind))))))

(fn build [context]
  "Build the declarations for keybindings."
  (let [declarations []]
    (var configured (or (and (= (type context.config) :table)
                             context.config.keybindings)
                        nil))
    (set configured (or (and (= (type configured) :table) configured) {}))
    (table.insert declarations
                  {:catalog :services
                   :id :keybindings.action
                   :value (fn [context-name event]
                            "Resolve a terminal input event to an action in a binding context."
                            (let [key (key-of event)]
                              (var matched nil)
                              (each [_ binding (ipairs (misa.keybindings.all))]
                                (when (= binding.context context-name)
                                  (let [section (or (and (= (type (. configured
                                                                     context-name))
                                                            :table)
                                                         (. configured
                                                            context-name))
                                                    {})]
                                    (var keys (. section binding.action))
                                    (when (= keys nil)
                                      (set keys binding.default))
                                    (when (= (type keys) :string)
                                      (set keys [keys]))
                                    (assert (= (type keys) :table)
                                            "configured keybinding must be a string or array")
                                    (each [_ candidate (ipairs keys)]
                                      (when (= candidate key)
                                        (assert (or (= matched nil)
                                                    (= matched binding.action))
                                                "ambiguous keybinding in context")
                                        (set matched binding.action))))))
                              matched))})
    (table.insert declarations
                  {:catalog :services
                   :id :keybindings.hint
                   :value (fn [context-name action]
                            "Return the first configured key for an action."
                            (let [section (or (and (= (type (. configured
                                                               context-name))
                                                      :table)
                                                   (. configured context-name))
                                              {})]
                              (var keys (. section action))
                              (when (= keys nil)
                                (each [_ binding (ipairs (misa.keybindings.all))]
                                  (when (and (= binding.context context-name)
                                             (= binding.action action))
                                    (set keys binding.default)
                                    (lua :break))))
                              (if (= (type keys) :string) keys
                                  (or (and (= (type keys) :table) (. keys 1))
                                      nil))))})
    (let [symbols {:alt "⌥"
                   :arrow_down "↓"
                   :arrow_left "←"
                   :arrow_right "→"
                   :arrow_up "↑"
                   :ctrl "⌃"
                   :enter "↵"
                   :escape :esc
                   :shift "⇧"
                   :space :space
                   :tab "⇥"}
          modifiers {:alt true :ctrl true :shift true}]
      (table.insert declarations
                    {:catalog :services
                     :id :keybindings.tokens
                     :value (fn [key]
                              "Split a key binding into display tokens."
                              (let [result {}
                                    normalized (: (tostring (or key "")) :gsub
                                                  "^ctrl_" "ctrl+")]
                                (each [part (: normalized :gmatch "[^+]+")]
                                  (let [text (or (. symbols part) part)]
                                    ;; Preserve the input spelling in every context, including
                                    ;; case-sensitive motions. Modifiers do not change case.
                                    (tset result (+ (length result) 1)
                                          {:kind (or (and (. modifiers part)
                                                          :modifier)
                                                     :key)
                                           : text})))
                                result))})
      (table.insert declarations
                    {:catalog :services
                     :id :keybindings.text
                     :value (fn [key]
                              "Format a key binding as readable text."
                              (let [parts {}]
                                (each [_ token (ipairs (misa.keybindings.tokens key))]
                                  (tset parts (+ (length parts) 1) token.text))
                                (table.concat parts)))})
      (table.insert declarations
                    {:catalog :services
                     :id :keybindings.render
                     :value (fn [key]
                              "Render a key binding as styled spans."
                              (let [spans {}]
                                (each [_ token (ipairs (misa.keybindings.tokens key))]
                                  (tset spans (+ (length spans) 1)
                                        {:style :keybinding
                                         :text token.text
                                         :token token.kind}))
                                spans))})
      (table.insert declarations
                    {:catalog :services
                     :id :keybindings.reference
                     :value (fn [entries]
                              "Render configured bindings for a context."
                              (let [spans {}]
                                (each [index entry (ipairs (or entries {}))]
                                  (when (> index 1)
                                    (tset spans (+ (length spans) 1)
                                          {:style :plain :text "   "}))
                                  (each [_ span (ipairs (misa.keybindings.render entry.key))]
                                    (set span.action entry.action)
                                    (tset spans (+ (length spans) 1) span))
                                  (tset spans (+ (length spans) 1)
                                        {:action entry.action
                                         :style :label
                                         :text (.. " " entry.label)}))
                                spans))})
      (definitions.build :keybindings declarations {}))))

{: build}
