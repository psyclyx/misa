;; The stock choices/picker module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.choices.picker.core))
(local render (require :misa.standard.choices.picker.render))
(local view (require :misa.standard.choices.picker.view))

(misa.merge-definitions [core render view])
