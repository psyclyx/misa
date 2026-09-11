;; The stock protocols module. This directory holds no declarations of its own, so
;; its module is exactly its children.
(local anthropic (require :misa.standard.protocols.anthropic))
(local openai (require :misa.standard.protocols.openai))

(misa.merge-definitions [anthropic openai])
