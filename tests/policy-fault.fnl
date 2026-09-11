(local definitions (require :tests.declarations))

;; An interactive session must survive a policy fault. "f" emits an invalid
;; native effect and "h" raises inside a handler; both are reported as runtime
;; events, and the view keeps rendering afterwards.
(fn [_context]
  (definitions.collect
    :tests.policy-fault
    [{:catalog :events
      :value {:event :app/start
              :handler (fn [_ _]
                         {:patch {:status "boot"} :fx [{:type :terminal/read}]})}}
     {:catalog :events
      :value {:event :terminal/input
              :handler (fn [_ event]
                         (match event
                           {:kind :text :text "f"} {:fx [{:type :not/native}]}
                           {:kind :text :text "h"} {:fx [{:type :dispatch
                                                          :event {:type :test/handler-fault}}]}
                           {:kind :text :text "c"} {:patch {:status "done"}
                                                    :fx [{:type :app/quit}]}
                           _ {:fx [{:type :terminal/read}]}))}}
     {:catalog :events
      :value {:event :test/handler-fault
              :handler (fn [_ _] (error "fixture handler exploded"))}}
     {:catalog :events
      :value {:event :runtime/effect-error
              :handler (fn [_ event]
                         (assert (string.find event.text "not/native" 1 true)
                                 "the notice did not name the effect")
                         {:patch {:status "effect-fault"}})}}
     {:catalog :events
      :value {:event :runtime/handler-error
              :handler (fn [_ event]
                         (assert (string.find event.text "exploded" 1 true)
                                 "the notice did not carry the failure")
                         {:patch {:status "handler-fault"}})}}]
    {:views {:main (fn [db _]
                     {:lines [{:spans [{:text (or db.status "boot")}]}]})}}))
