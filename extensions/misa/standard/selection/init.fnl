;; The stock selection module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.selection.core))
(local document (require :misa.standard.selection.document))
(local render (require :misa.standard.selection.render))

(misa.merge-definitions [core document render])
