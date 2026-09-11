;; The stock editor/queue module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.editor.queue.core))
(local view (require :misa.standard.editor.queue.view))

(misa.merge-definitions [core view])
