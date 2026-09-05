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

{:setup (fn [context]
          (local setup-fx [])
          (var configured (or (and (= (type context.config) :table)
                                   context.config.keybindings)
                              nil))
          (set configured (or (and (= (type configured) :table) configured) {}))
          (table.insert setup-fx
                        {:type :register/service
                         :name :keybinding_action
                         :value (fn [context-name event]
                                  (local key (key-of event))
                                  (each [_ binding (ipairs (misa.keybindings))]
                                    (when (= binding.context context-name)
                                      (local section
                                             (or (and (= (type (. configured
                                                                  context-name))
                                                         :table)
                                                      (. configured
                                                         context-name))
                                                 {}))
                                      (var keys (. section binding.action))
                                      (when (= keys nil)
                                        (set keys binding.default))
                                      (when (= (type keys) :string)
                                        (set keys [keys]))
                                      (assert (= (type keys) :table)
                                              "configured keybinding must be a string or array")
                                      (each [_ candidate (ipairs keys)]
                                        (when (= candidate key)
                                          (let [___antifnl_rtn_1___ binding.action]
                                            (lua "return ___antifnl_rtn_1___"))))))
                                  nil)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :keybinding_hint
                         :value (fn [context-name action]
                                  (local section
                                         (or (and (= (type (. configured
                                                              context-name))
                                                     :table)
                                                  (. configured context-name))
                                             {}))
                                  (var keys (. section action))
                                  (when (= keys nil)
                                    (each [_ binding (ipairs (misa.keybindings))]
                                      (when (and (= binding.context
                                                    context-name)
                                                 (= binding.action action))
                                        (set keys binding.default)
                                        (lua :break))))
                                  (if (= (type keys) :string) keys
                                      (or (and (= (type keys) :table)
                                               (. keys 1))
                                          nil)))})
          (local symbols {:alt "⌥"
                          :arrow_down "↓"
                          :arrow_left "←"
                          :arrow_right "→"
                          :arrow_up "↑"
                          :ctrl "⌃"
                          :ctrl_n "⌃N"
                          :ctrl_p "⌃P"
                          :ctrl_r "⌃R"
                          :ctrl_v "⌃V"
                          :enter "↵"
                          :escape :esc
                          :shift "⇧"
                          :space :space
                          :tab "⇥"})
          (local modifiers {:alt true :ctrl true :shift true})
          (table.insert setup-fx
                        {:type :register/service
                         :name :keybinding_tokens
                         :value (fn [key]
                                  (local result {})
                                  (var alt false)
                                  (each [part (: (tostring (or key "")) :gmatch
                                                 "[^+]+")]
                                    (var text (or (. symbols part) part))
                                    (when (= part :alt) (set alt true))
                                    (when (or (and (and alt (= (length text) 1))
                                                   (text:match "%l"))
                                              (text:match "^f%d+$"))
                                      (set text (text:upper)))
                                    (tset result (+ (length result) 1)
                                          {:kind (or (and (. modifiers part)
                                                          :modifier)
                                                     :key)
                                           : text}))
                                  result)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :keybinding_text
                         :value (fn [key]
                                  (local parts {})
                                  (each [_ token (ipairs (misa.keybinding_tokens key))]
                                    (tset parts (+ (length parts) 1) token.text))
                                  (table.concat parts))})
          (table.insert setup-fx
                        {:type :register/service
                         :name :render_keybinding
                         :value (fn [key]
                                  (local spans {})
                                  (each [_ token (ipairs (misa.keybinding_tokens key))]
                                    (tset spans (+ (length spans) 1)
                                          {:style :keybinding
                                           :text token.text
                                           :token token.kind}))
                                  spans)})
          (table.insert setup-fx
                        {:type :register/service
                         :name :render_keybinding_reference
                         :value (fn [entries]
                                  (local spans {})
                                  (each [index entry (ipairs (or entries {}))]
                                    (when (> index 1)
                                      (tset spans (+ (length spans) 1)
                                            {:style :plain :text "   "}))
                                    (each [_ span (ipairs (misa.render_keybinding entry.key))]
                                      (set span.action entry.action)
                                      (tset spans (+ (length spans) 1) span))
                                    (tset spans (+ (length spans) 1)
                                          {:action entry.action
                                           :style :label
                                           :text (.. " " entry.label)}))
                                  spans)})
          ;; The native decoder already distinguishes standalone Escape from Alt
          ;; chords. Normalizing only completed Alt events avoids swallowing Escape
          ;; while waiting for a byte that may never arrive.
          (table.insert setup-fx
                        {:type :register/interceptor
                         :value {:before (fn [tx]
                                           (when (and (and (= tx.event.type
                                                              :terminal/input)
                                                           (= tx.event.kind
                                                              :alt))
                                                      (= (type tx.event.text)
                                                         :string))
                                             (set tx.event
                                                  {:key (.. :alt+
                                                            (tx.event.text:lower))
                                                   :kind :key
                                                   :type :terminal/input}))
                                           tx)
                                 :id :keybindings/normalize-alt}})
          {:fx setup-fx})}
