(local {: render} (require :misa.dialogs.render))

{:components {:default.dialog {:compose true : render}}
 :requirements {:component.dialog [:layout :components.buttons]}}
