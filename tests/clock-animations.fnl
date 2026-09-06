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

{:setup (fn [context]
          {:fx [{:type :register/event
                 :name :app/start
                 :handler (fn [db]
                            (set db.mode :active)
                            (set db.ticks 0)
                            (set db.gates 0)
                            {: db
                             :fx (if context.config.headless
                                     [{:type :view/commit
                                       :lines (. (view db) :lines)}
                                      {:type :app/quit}]
                                     [{:type :terminal/read}])})}
                {:type :register/event
                 :name :animations/tick
                 :handler (fn [db]
                            (set db.ticks (+ db.ticks 1))
                            {: db})}
                {:type :register/event
                 :name :terminal/input
                 :handler (fn [db event]
                            (if (= event.kind :ctrl_d)
                                {:fx [{:type :app/quit}]}
                                (do
                                  (match event.kind
                                    :ctrl_r (do
                                              ;; Deterministic test-only gate.
                                              ((assert (loadfile context.config.gate)))
                                              (set db.gates (+ db.gates 1)))
                                    :tab (set db.mode :static)
                                    :backspace (set db.mode :removed))
                                  {: db :fx [{:type :terminal/read}]})))}
                {:type :register/view :handler view}]})}
