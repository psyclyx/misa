(local {: render-picker} (require :misa.choices.picker.render))

{:components {:default.picker {:render render-picker}}
 :requirements {:component.picker [:layout]}}
