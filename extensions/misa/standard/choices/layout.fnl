(local {: choices-completion-layout
        : choices-row-lines
        : choices-viewport
        : overlay-options
        : choices-picker-layout} (require :misa.choices.layout))

{:services {:choices.row-lines choices-row-lines
            :choices.viewport choices-viewport
            :choices.picker-layout (fn [previous db terminal]
                                     (choices-picker-layout (overlay-options (misa.configuration))
                                                            previous db terminal))
            :choices.completion-layout choices-completion-layout}
 :requirements {:choice_layout [:choices.preview
                                :choices.projected-rows
                                :layout]}}
