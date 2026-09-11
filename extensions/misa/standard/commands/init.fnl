;; The stock commands module: this directory's own declarations, merged with
;; its children.
(local core (require :misa.standard.commands.core))
(local palette (require :misa.standard.commands.palette))

(misa.merge-definitions [core palette])
