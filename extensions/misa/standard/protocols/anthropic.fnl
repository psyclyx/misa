(local protocol (require :misa.protocols.anthropic))
(fn function-definition [_ value]
  (assert (= (type value) :function) "record projection must be a function"))

{:services {:protocols.anthropic-messages protocol.messages}
 :anthropic-records protocol.records
 :anthropic-block-starts protocol.starts
 :anthropic-block-deltas protocol.deltas
 :validators {:anthropic-records function-definition
              :anthropic-block-starts function-definition
              :anthropic-block-deltas function-definition}}
