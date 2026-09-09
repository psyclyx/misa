(local ui (require :misa.ui))
(local layout (require :misa.ui.layout))
{:services {:ui.regions ui.regions
            :ui.input-budgets ui.input-budgets
            :ui.overlay-room ui.overlay-room
            :ui.bound-frame ui.bound-frame
            :ui.picker-room ui.ui-picker-room
            :ui.completion-room ui.ui-completion-room
            :layout layout}
 :views {:main ui.main}}
