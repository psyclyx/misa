(local {: attachments-lines : on-draft-attachments : render-controls}
       (require :misa.editor.attachments))

{:components {:default.attachment-controls {:render render-controls}}
 :services {:attachments.lines attachments-lines}
 :view-layers {:draft-attachments {:handler on-draft-attachments}}}
