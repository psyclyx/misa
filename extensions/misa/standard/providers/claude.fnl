(local claude (require :misa.providers.claude))
(local serializer :claude.cli)
(local api {:request_options {:reasoning_effort {:choices [:low
                                                           :medium
                                                           :high
                                                           :max]
                                                 :default :high}}
            :request_options_serializer serializer})

(fn settings []
  (or (. (or (. (misa.configuration) :providers) {}) :claude) {}))

(fn function-definition [_ value]
  (assert (= (type value) :function) "record projection must be a function"))

(fn provider-models [db]
  (icollect [_ model (ipairs (or (and db.models db.models.catalogue) []))]
    (when (= model.provider :claude)
      model)))

{:auth-providers {:claude {:description "Claude Pro/Max via Claude Code"
                           :id :claude
                           :label :Claude
                           :model_provider :claude
                           :strategy :cli_handoff}}
 :serializers {serializer {:accepts (fn [name] (= name :reasoning_effort))
                           :serialize (fn [name value]
                                        (when (= name :reasoning_effort)
                                          [:--effort value]))}}
 :effects {:provider.claude (fn [effect cofx]
                              (let [config (settings)]
                                (claude.request config serializer
                                                (or config.executable :claude)
                                                effect cofx)))}
 :events {:provider.claude/usage-refresh {:event :usage/refresh
                                          :handler (fn [db event]
                                                     (let [config (settings)]
                                                       (claude.refresh-usage config
                                                                             (or config.executable
                                                                                 :claude)
                                                                             db
                                                                             event)))
                                          :priority 10000}
          :provider.claude/usage {:event :provider/claude-usage
                                  :handler claude.receive-usage
                                  :priority 10000}
          :provider.claude/availability {:event :models/provider-availability
                                         :handler (fn [db event]
                                                    (claude.provider-availability (provider-models db)
                                                                                  db
                                                                                  event))
                                         :priority 10000}
          :provider.claude/complete {:event :provider/claude-complete
                                     :handler claude.stream
                                     :priority 10000}
          :provider.claude/quota {:event :provider/claude-quota
                                  :handler claude.receive-quota
                                  :priority 10000}}
 :claude-records claude.records
 :claude-stream-events claude.partials
 :validators {:claude-records function-definition
              :claude-stream-events function-definition}
 :models {:claude/claude-fable-5-1 {:context_window 1000000
                                    :id :claude/claude-fable-5-1
                                    :label "Claude Fable 5.1"
                                    :model :claude-fable-5-1
                                    : api
                                    :provider :claude}
          :claude/claude-opus-5 {:context_window 200000
                                 :id :claude/claude-opus-5
                                 :label "Claude Opus 5"
                                 :model :claude-opus-5
                                 : api
                                 :provider :claude}
          :claude/claude-sonnet-5 {:context_window 1000000
                                   :id :claude/claude-sonnet-5
                                   :label "Claude Sonnet 5"
                                   :model :claude-sonnet-5
                                   : api
                                   :provider :claude}
          :claude/claude-haiku-4-5-20251001 {:context_window 200000
                                             :id :claude/claude-haiku-4-5-20251001
                                             :label "Claude Haiku 4.5"
                                             :model :claude-haiku-4-5-20251001
                                             : api
                                             :provider :claude}}}
