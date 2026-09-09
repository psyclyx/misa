(local {: on-pending-prompt : render-pending} (require :misa.editor.queue.view))

{:components {:default.pending-prompt {:render render-pending}}
 :view-layers {:pending-prompt {:handler on-pending-prompt}}}
