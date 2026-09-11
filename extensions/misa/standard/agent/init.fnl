;; The stock agent module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.agent.core))
(local stream (require :misa.standard.agent.stream))

(misa.merge-definitions [core stream])
