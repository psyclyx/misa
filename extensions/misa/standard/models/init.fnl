;; The stock models module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.models.core))
(local effort (require :misa.standard.models.effort))
(local options (require :misa.standard.models.options))
(local preview (require :misa.standard.models.preview))

(misa.merge-definitions [core effort options preview])
