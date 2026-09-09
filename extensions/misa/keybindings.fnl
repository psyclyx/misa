(local definitions (require :misa.definitions))

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

(fn binding-keys [configured binding]
  (let [raw (. configured binding.context)
        section (if (= (type raw) :table) raw {})
        override (. section binding.action)
        keys (if (= override nil) binding.default override)
        normalized (if (= (type keys) :string) [keys] keys)]
    (assert (= (type normalized) :table)
            "configured keybinding must be a string or array")
    normalized))

(fn resolve-action [configured bindings context-name event]
  "Resolve an input event against configured bindings, rejecting ambiguity."
  (let [key (key-of event)]
    (accumulate [matched nil _ binding (ipairs bindings)]
      (if (= binding.context context-name)
          (accumulate [found matched _ candidate (ipairs (binding-keys configured
                                                                       binding))]
            (if (= candidate key)
                (do
                  (assert (or (= found nil) (= found binding.action))
                          "ambiguous keybinding in context")
                  binding.action)
                found))
          matched))))

(fn hint [configured bindings context-name action]
  (let [binding (accumulate [found nil _ binding (ipairs bindings) &until found]
                  (when (and (= binding.context context-name)
                             (= binding.action action))
                    binding))
        raw (. configured context-name)
        override (and (= (type raw) :table) (. raw action))]
    (if binding (. (binding-keys configured binding) 1)
        (= (type override) :string) override
        (= (type override) :table) (. override 1))))

(fn tokens [key]
  "Split a key binding into display tokens, preserving key case."
  (let [normalized (: (tostring (or key "")) :gsub "^ctrl_" "ctrl+")]
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

(fn build [context]
  "Declare configurable semantic keybindings and their display services."
  (let [raw (and (= (type context.config) :table) context.config.keybindings)
        configured (if (= (type raw) :table) raw {})]
    (definitions.build :keybindings
      [{:catalog :services
        :id :keybindings.action
        :value (fn [context-name event]
                 "Resolve an input event in a binding context."
                 (resolve-action configured (misa.keybindings.all) context-name
                                 event))}
       {:catalog :services
        :id :keybindings.hint
        :value (fn [context-name action]
                 "Return the first configured key for an action."
                 (hint configured (misa.keybindings.all) context-name action))}
       {:catalog :services :id :keybindings.tokens :value tokens}
       {:catalog :services :id :keybindings.text :value text}
       {:catalog :services :id :keybindings.render :value render}
       {:catalog :services :id :keybindings.reference :value reference}])))

{: build : resolve-action : tokens}
