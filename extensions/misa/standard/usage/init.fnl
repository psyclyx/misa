;; The stock usage module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.usage.core))
(local dialog (require :misa.standard.usage.dialog))

(misa.merge-definitions [core dialog])
