;; Static application chrome; dynamic model/session facts belong in status.

(fn default-root-header-render []
  "Render the default application header."
  {:lines [{:spans [{:style :bold :text :misa}
                    {:style :dim :text "  ·  coding agent"}]}
           {:spans [{:style :dim :text "/ conversation   : actions   F1 help"}]}]})

{: default-root-header-render}
