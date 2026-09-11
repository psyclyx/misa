;; The stock presentation module. This directory holds no declarations of its own, so
;; its module is exactly its children.
(local animations (require :misa.standard.presentation.animations))
(local components (require :misa.standard.presentation.components))
(local elements (require :misa.standard.presentation.elements))
(local markdown (require :misa.standard.presentation.markdown))
(local status (require :misa.standard.presentation.status))
(local syntax (require :misa.standard.presentation.syntax))
(local theme (require :misa.standard.presentation.theme))
(local tools (require :misa.standard.presentation.tools))
(local transcript (require :misa.standard.presentation.transcript))
(local ui (require :misa.standard.presentation.ui))
(local values-catalog (require :misa.standard.presentation.values))

(misa.merge-definitions [animations
                         components
                         elements
                         markdown
                         status
                         syntax
                         theme
                         tools
                         transcript
                         ui
                         values-catalog])
