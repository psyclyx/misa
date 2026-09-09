(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(app.add :misa.usage)
(app.add :misa.usage.dialog)
(local requests [])
(var observed nil)
(app.define {:events {:fixture/select {:event :model/select
                                       :handler (fn [_ event]
                                                  {:patch {:selected (misa.replace event.model)}})}
                      :fixture/refresh {:event :usage/refresh
                                        :handler (fn [_ event]
                                                   (table.insert requests
                                                                 event.provider)
                                                   nil)}
                      :fixture/read {:event :test/read
                                     :handler (fn [db] (set observed db) nil)}}})

(app.install)
(set misa.models.selected (fn [db] db.selected))
(fn dispatch [event]
  (local effects (misa._dispatch event
                                 {:columns 80 :lines 24 :interactive false}
                                 {:wall_ms 0 :monotonic_ms 0}))
  (misa._commit)
  (each [_ effect (ipairs effects)]
    (when (= effect.type :dispatch) (dispatch effect.event))))

(dispatch {:type :app/start})
(dispatch {:type :model/select :model {:provider :first}})
(assert (= (length requests) 1))
(assert (= (. requests 1) :first))
(dispatch {:type :agent/usage :usage {:input_tokens 12 :output_tokens 8}})
(dispatch {:type :agent/status :status :working :last_usage {:input_tokens 3}})
(dispatch {:type :test/read})
(assert (= observed.status nil) "usage installed status presentation state")
(assert (= (. (misa.catalog :subscriptions) :status/session) nil))
(assert (= (. (misa.sub observed [:usage/session]) :output_tokens) 8))
(assert (= (. (misa.sub observed [:usage/last-request]) :input_tokens) 3))
(dispatch {:type :model/select :model {:provider :first}})
(assert (= (length requests) 1) "unchanged provider refreshed quotas")
(dispatch {:type :auth/ready})
(assert (= (length requests) 2) "usage refresh depended on status UI")
(dispatch {:type :model/select :model {:provider :second}})
(assert (= (. requests 3) :second) "refresh observed the old selection")
(output "usage lifecycle is independent of status UI\n")
