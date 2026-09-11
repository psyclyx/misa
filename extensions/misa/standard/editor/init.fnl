;; The stock editor module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.editor.core))
(local attachments (require :misa.standard.editor.attachments))
(local editing (require :misa.standard.editor.editing))
(local history (require :misa.standard.editor.history))
(local images (require :misa.standard.editor.images))
(local queue (require :misa.standard.editor.queue))
(local render (require :misa.standard.editor.render))

(misa.merge-definitions [core attachments editing history images queue render])
