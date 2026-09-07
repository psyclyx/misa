;; Static application chrome; dynamic model/session facts belong in status.

{:setup (fn []
          {:fx [{:type :register/component
                 :id :default.root.header
                 :value {:render (fn []
                                   {:lines [{:spans [{:style :bold :text :misa}
                                                     {:style :dim
                                                      :text "  ·  coding agent"}]}
                                            {:spans [{:style :dim
                                                      :text "/ conversation   : actions   F1 help"}]}]})}}]})}
