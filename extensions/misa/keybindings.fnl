(local symbols {:alt "⌥"
                :arrow_down "↓"
                :arrow_left "←"
                :arrow_right "→"
                :arrow_up "↑"
                :ctrl "⌃"
                :enter "↵"
                :escape :esc
                :shift "⇧"
                :space :space
                :tab "⇥"})

(local modifiers {:alt true :ctrl true :shift true})

(fn key-of [event]
  (if (= event.kind :alt_enter) :alt+enter
      (= event.kind :shift_enter) :shift+enter
      (= event.kind :key) event.key
      (= event.kind :text) event.text
      (and (= event.kind :alt) (= (type event.text) :string)) (.. :alt+
                                                                  (event.text:lower))
      event.kind))

(fn binding-keys [binding]
  (let [keys binding.default
        normalized (if (= (type keys) :string) [keys] keys)]
    (assert (= (type normalized) :table) "keybinding must be a string or array")
    normalized))

(fn resolve-action [bindings context-name event]
  "Resolve an input event against bindings, rejecting ambiguity."
  (let [key (key-of event)]
    (accumulate [matched nil _ binding (ipairs bindings)]
      (if (= binding.context context-name)
          (accumulate [found matched _ candidate (ipairs (binding-keys binding))]
            (if (= candidate key)
                (do
                  (assert (or (= found nil) (= found binding.action))
                          "ambiguous keybinding in context")
                  binding.action)
                found))
          matched))))

(fn hint [bindings context-name action]
  "Return the first key for an action."
  (let [binding (accumulate [found nil _ binding (ipairs bindings) &until found]
                  (when (and (= binding.context context-name)
                             (= binding.action action))
                    binding))]
    (when binding (. (binding-keys binding) 1))))

(fn tokens [key]
  "Split a key binding into display tokens, preserving key case."
  (let [normalized (: (tostring (or key "")) :gsub :^ctrl_ :ctrl+)]
    (icollect [part (: normalized :gmatch "[^+]+")]
      {:kind (if (. modifiers part) :modifier :key)
       :text (or (. symbols part) part)})))

(fn text [key]
  "Format a key binding as readable text."
  (table.concat (icollect [_ token (ipairs (misa.keybindings.tokens key))]
                  token.text)))

(fn render [key]
  "Render a key binding as styled spans."
  (icollect [_ token (ipairs (misa.keybindings.tokens key))]
    {:style :keybinding :text token.text :token token.kind}))

(fn reference [entries]
  "Render labeled bindings and their actions as spans."
  (let [spans []]
    (each [index entry (ipairs (or entries []))]
      (when (< 1 index) (table.insert spans {:style :plain :text "   "}))
      (each [_ span (ipairs (misa.keybindings.render entry.key))]
        (table.insert spans (misa.patch span {:action entry.action})))
      (table.insert spans
                    {:action entry.action
                     :style :label
                     :text (.. " " entry.label)}))
    spans))

{: resolve-action : tokens : hint : text : render : reference}
