(local definitions (require :misa.definitions))

;; Static application chrome; dynamic model/session facts belong in status.

(fn build []
  "Declare application header rendering."
  (definitions.build :component.chrome
    [{:catalog :components
      :id :default.root.header
      :value {:render (fn []
                        {:lines [{:spans [{:style :bold :text :misa}
                                          {:style :dim
                                           :text "  ·  coding agent"}]}
                                 {:spans [{:style :dim
                                           :text "/ conversation   : actions   F1 help"}]}]})}}]
    {}))

{: build}
