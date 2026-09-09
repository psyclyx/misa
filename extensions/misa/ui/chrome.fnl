(local definitions (require :misa.definitions))

;; Static application chrome; dynamic model/session facts belong in status.

(fn default-root-header-render []
  {:lines [{:spans [{:style :bold :text :misa}
                    {:style :dim :text "  ·  coding agent"}]}
           {:spans [{:style :dim :text "/ conversation   : actions   F1 help"}]}]})

(fn build []
  "Declare application header rendering."
  (definitions.build :component.chrome
    [{:catalog :components
      :id :default.root.header
      :value {:render default-root-header-render}}]
    {}))

{:build build}
