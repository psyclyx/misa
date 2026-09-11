;; The stock tools module. This directory holds no declarations of its own, so
;; its module is exactly its children.
(local files (require :misa.standard.tools.files))
(local shell (require :misa.standard.tools.shell))
(local web-search (require :misa.standard.tools.web-search))

(misa.merge-definitions [files shell web-search])
