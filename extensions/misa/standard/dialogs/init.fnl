;; The stock dialogs module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.dialogs.core))
(local render (require :misa.standard.dialogs.render))
(local view (require :misa.standard.dialogs.view))

(misa.merge-definitions [core render view])
