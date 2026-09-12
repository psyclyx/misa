;; Stock application plus the test-only fake provider. One response asks for
;; more tools than a single journal append accepts, so the turn's canonical
;; history has to be recorded in more than one bounded append. Each append
;; reports what the store wrote, which is what the case asserts.
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
  (tset app.definitions.events :big-journal/appended
        {:event :conversation/appended
         :handler (fn [_ event]
                    (when event.ok
                      {:fx [{:lines [{:spans [{:text (.. "appended "
                                                         (tostring (. event.data
                                                                      :count)))}]}]
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
