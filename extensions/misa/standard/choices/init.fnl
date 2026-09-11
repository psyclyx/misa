;; The stock choices module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.choices.core))
(local layout (require :misa.standard.choices.layout))
(local matching (require :misa.standard.choices.matching))
(local picker (require :misa.standard.choices.picker))
(local preferences (require :misa.standard.choices.preferences))
(local preview (require :misa.standard.choices.preview))

(misa.merge-definitions [core layout matching picker preferences preview])
