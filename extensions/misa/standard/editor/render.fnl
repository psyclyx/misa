(local {: render-completions : render-input} (require :misa.editor.render))

{:components {:default.editor.input {:render render-input}
              :default.editor.completions {:render render-completions}}
 :requirements {:component.editor [:layout :layout.wrap-input]}}
