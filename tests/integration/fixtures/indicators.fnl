{:setup (fn []
          (misa.reg_keybinding {:action :cycle
                                :context :test
                                :default [:alt+x]})
          (misa.reg_indicator {:icon "!"
                               :id :important
                               :label :Important
                               :value (fn [] :yes)})
          (misa.reg_indicator {:hotkey {:action :cycle :context :test}
                               :icon "?"
                               :id :optional
                               :label :Optional
                               :value (fn [] :wide)})
          (misa.reg_event :app/start
                          (fn [db]
                            (local wide
                                   (. (misa.indicators_projection db
                                                                  {:columns 80})
                                      1 :spans))
                            (var (labels ___values___ hotkey-text)
                                 (values 0 0 ""))
                            (each [_ item (ipairs wide)]
                              (when (and (= item.style.dim true)
                                         (= item.text :Important))
                                (set labels (+ labels 1)))
                              (when (and (= item.style.foreground :default)
                                         (= item.text :yes))
                                (set ___values___ (+ ___values___ 1)))
                              (local accent
                                     (. (misa.theme_style db :keybinding)
                                        :foreground))
                              (local foreground item.style.foreground)
                              (when (and (and (and (and (= item.style.dim true)
                                                        (= (type foreground)
                                                           :table))
                                                   (= foreground.r accent.r))
                                              (= foreground.g accent.g))
                                         (= foreground.b accent.b))
                                (set hotkey-text (.. hotkey-text item.text))))
                            (assert (and (and (= labels 1) (= ___values___ 1))
                                         (= hotkey-text "⌥X"))
                                    "indicator semantic classes or structured hotkey are missing")
                            (local narrow
                                   (. (misa.indicators_projection db
                                                                  {:columns 15})
                                      1 :spans))
                            (var text "")
                            (each [_ item (ipairs narrow)]
                              (set text (.. text item.text)))
                            (assert (and (text:find :Important 1 true)
                                         (not (text:find :Optional 1 true)))
                                    "indicator priority did not control narrow-width dropping")
                            {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                     :text :indicators}]}]
                                   :type :view/commit}
                                  {:type :app/quit}]}))
          nil)}

