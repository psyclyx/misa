;; Stock application plus the test-only fake provider. One response asks for
;; more tools than a single append would accept, so the turn's canonical history
;; has to be written as each message settles rather than in one batch. The
;; handler reports the running count and how many appends produced it, which is
;; what the case asserts.
(let [app (misa.snapshot (require :misa.standard))
      fake (require :misa.standard.providers.fake)]
  (each [kind entries (pairs fake)]
    (each [id value (pairs entries)]
      (tset (. app.definitions kind) id value)))
  (var stream [])
  (for [index 1 257]
    (table.insert stream {:arguments {}
                          :id (.. :call- (tostring index))
                          :name :no-such-tool
                          :type :tool_call}))
  (var recorded 0)
  (var appends 0)
  (tset app.definitions.events :big-journal/appended
        {:event :conversation/appended
         :handler (fn [_ event]
                    (when event.ok
                      (set recorded (+ recorded (or (. event.data :count) 0)))
                      (set appends (+ appends 1))
                      {:fx [{:lines [{:spans [{:text (.. "recorded "
                                                         (tostring recorded)
                                                         " in "
                                                         (tostring appends)
                                                         " appends")}]}]
                             :type :view/commit}]}))
         :priority 12000})
  (tset app.config :runtime
        (misa.patch (or app.config.runtime {}) {:max_dispatch_chain 1000000}))
  (tset app.config :models
        (misa.patch (or app.config.models {}) {:default :fake/default}))
  (tset app.config :providers
        (misa.patch (or app.config.providers {})
                    {:fake {:responses [{: stream}]}}))
  app)
