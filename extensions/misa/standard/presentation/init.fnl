(local groups [(require :misa.standard.presentation.markdown)
               (require :misa.standard.presentation.ui)
               (require :misa.standard.presentation.values)
               (require :misa.standard.presentation.elements)
               (require :misa.standard.presentation.components)
               (require :misa.standard.presentation.theme)
               (require :misa.standard.presentation.animations)
               (require :misa.standard.presentation.transcript)
               (require :misa.standard.presentation.syntax)
               (require :misa.standard.presentation.tools)
               (require :misa.standard.presentation.status)])

(local catalogs {})
(each [_ group (ipairs groups)]
  (each [kind entries (pairs group)]
    (let [target (or (. catalogs kind) {})]
      (tset catalogs kind target)
      (each [id value (pairs entries)] (tset target id value)))))

catalogs
