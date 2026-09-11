(local definitions (require :tests.declarations))

(fn []
  (local declarations [])
  (table.insert declarations
                (let [definition {:action :cycle
                                  :context :test
                                  :default [:alt+x]}]
                  {:catalog :keybindings
                   :id (.. (. definition :context) "/" (. definition :action))
                   :value definition}))
  (table.insert declarations
                (let [definition {:icon "!"
                                  :id :important
                                  :label :Important
                                  :query [:test/important]}]
                  {:catalog :indicators
                   :id (. definition :id)
                   :value definition}))
  (table.insert declarations
                (let [definition {:id :test/important
                                  :inputs []
                                  :compute (fn [] {:type :text :value :yes})}]
                  {:catalog :subscriptions
                   :id (. definition :id)
                   :value definition}))
  (table.insert declarations
                (let [definition {:hotkey {:action :cycle :context :test}
                                  :icon "?"
                                  :id :optional
                                  :label :Optional
                                  :query [:test/optional]}]
                  {:catalog :indicators
                   :id (. definition :id)
                   :value definition}))
  (table.insert declarations
                (let [definition {:id :test/optional
                                  :inputs []
                                  :compute (fn [] {:type :text :value :wide})}]
                  {:catalog :subscriptions
                   :id (. definition :id)
                   :value definition}))
  (table.insert declarations
                {:catalog :events
                 :value {:event :app/start
                         :handler (fn [db]
                                    (local wide
                                           (. (misa.status.indicators db
                                                                      {:columns 80})
                                              1 :spans))
                                    (var (labels ___values___ hotkey-text)
                                         (values 0 0 ""))
                                    (each [_ item (ipairs wide)]
                                      (when (and (= item.style.dim true)
                                                 (= item.text :Important))
                                        (set labels (+ labels 1)))
                                      (when (and (= item.style.foreground
                                                    :default)
                                                 (= item.text :yes))
                                        (set ___values___ (+ ___values___ 1)))
                                      (local accent
                                             (. (misa.themes.style db
                                                                   :keybinding)
                                                :foreground))
                                      (local foreground item.style.foreground)
                                      (when (and (and (and (and (= item.style.dim
                                                                   true)
                                                                (= (type foreground)
                                                                   :table))
                                                           (= foreground.r
                                                              accent.r))
                                                      (= foreground.g accent.g))
                                                 (= foreground.b accent.b))
                                        (set hotkey-text
                                             (.. hotkey-text item.text))))
                                    (assert (and (and (= labels 1)
                                                      (= ___values___ 1))
                                                 (= hotkey-text "⌥x"))
                                            "indicator semantic classes or structured hotkey are missing")
                                    (local narrow
                                           (. (misa.status.indicators db
                                                                      {:columns 15})
                                              1 :spans))
                                    (var text "")
                                    (each [_ item (ipairs narrow)]
                                      (set text (.. text item.text)))
                                    (assert (and (text:find :Important 1 true)
                                                 (not (text:find :Optional 1
                                                                 true)))
                                            "indicator priority did not control narrow-width dropping")
                                    {:fx [{:lines [{:spans [{:style {:foreground :default}
                                                             :text :indicators}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})}})
  nil
  (definitions.collect :tests.integration.fixtures.indicators declarations {}))
