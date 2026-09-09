(local {: choice-previews : line : metadata : choices-preview}
       (require :misa.choices.preview))

{:choice-previews {: metadata :text (fn [preview] [(line preview.value)])}
 :services {:choices.preview choices-preview}
 :validators {: choice-previews}}
