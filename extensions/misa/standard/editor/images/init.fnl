;; The stock editor/images module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.editor.images.core))
(local render (require :misa.standard.editor.images.render))

(misa.merge-definitions [core render])
