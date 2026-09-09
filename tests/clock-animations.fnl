(local definitions (require :tests.declarations))

;; Raw native descriptors isolate presentation clocks from Fennel timer policy.
(fn view [db]
  (local active (= db.mode :active))
  (local moving {:text :ABC
                 :style {:bold true}
                 :link "https://example.test/animation"})
  (local glowing {:text :GLOW :style {:underline true}})
  (when active
    (set moving.animation
         {:id :fixture/text
          :interval_ms 40
          :frames [{:text :ABC} {:text :DEF}]})
    ;; Omitted frame text inherits the base text; style overrides retain underline.
    (set glowing.animation
         {:id :fixture/style
          :interval_ms 40
          :frames [{:style {:foreground :red}} {:style {:foreground :green}}]}))
  (when (= db.mode :static)
    (set moving.animation {:id :fixture/text
                           :interval_ms 40
                           :frames [{:text :ABC}]}))
  (when (= db.mode :removed)
    (set moving.text :END)
    (set glowing.text :DONE))
  {:lines [{:spans [moving
                    {:text " "}
                    glowing
                    {:text (.. " MODE=" (or db.mode "") " ticks="
                               (or db.ticks 0) " gates=" (or db.gates 0))}]}]
   :cursor {:row 1 :byte 0}})

(fn [context]
          (definitions.collect :tests.clock-animations [{:catalog :events  :value {:event :app/start :handler (fn [db]
                            (local initial {:mode :active :ticks 0 :gates 0})
                            {:patch initial
                             :fx (if context.config.headless
                                     [{:type :view/commit
                                       :lines (. (view (misa.patch db initial)) :lines)}
                                      {:type :app/quit}]
                                     [{:type :terminal/read}])})}}
                {:catalog :events  :value {:event :animations/tick :handler (fn [db]
                            {:patch {:ticks (+ db.ticks 1)}})}}
                {:catalog :events  :value {:event :terminal/input :handler (fn [db event]
                            (if (= event.kind :ctrl_d)
                                {:fx [{:type :app/quit}]}
                                (do
                                  (local patch (match event.kind
                                    :ctrl_r (do
                                              ;; Deterministic test-only gate.
                                              ((assert (loadfile context.config.gate)))
                                              {:gates (+ db.gates 1)})
                                    :tab {:mode :static}
                                    :backspace {:mode :removed}
                                    _ {}))
                                  {: patch :fx [{:type :terminal/read}]})))}}
                {:catalog :views :id :main :value view}] {}))
