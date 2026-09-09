(local protocol (require :misa.protocols.openai))
(fn function-definition [_ value]
  (assert (= (type value) :function) "record projection must be a function"))

{:services {:protocols.openai-messages protocol.messages}
 :openai-deltas (collect [_ projection (ipairs protocol.delta-projections)]
                  projection.id
                  projection.project)
 :validators {:openai-deltas function-definition}}
