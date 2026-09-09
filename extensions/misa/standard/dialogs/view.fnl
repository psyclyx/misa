(local {: projection} (require :misa.dialogs.view))

{:services {:dialogs.layout projection}
 :view-layers {:dialog {:handler projection}}
 :requirements {:dialog_view [:dialogs.enabled?]}}
