;; The stock combined application. Each entry is a direct child of this
;; directory, and nothing here reaches past its immediate children.
(local settings (require :misa.standard.settings))
(local actions (require :misa.standard.actions))
(local agent (require :misa.standard.agent))
(local choices (require :misa.standard.choices))
(local clipboard (require :misa.standard.clipboard))
(local commands (require :misa.standard.commands))
(local compaction (require :misa.standard.compaction))
(local conversation (require :misa.standard.conversation))
(local costs (require :misa.standard.costs))
(local dialogs (require :misa.standard.dialogs))
(local editor (require :misa.standard.editor))
(local json (require :misa.standard.json))
(local keybindings (require :misa.standard.keybindings))
(local links (require :misa.standard.links))
(local models (require :misa.standard.models))
(local presentation (require :misa.standard.presentation))
(local protocols (require :misa.standard.protocols))
(local providers (require :misa.standard.providers))
(local selection (require :misa.standard.selection))
(local state (require :misa.standard.state))
(local tools (require :misa.standard.tools))
(local usage (require :misa.standard.usage))

{:config settings
 :definitions (misa.merge-definitions [actions
                                       agent
                                       choices
                                       clipboard
                                       commands
                                       compaction
                                       conversation
                                       costs
                                       dialogs
                                       editor
                                       json
                                       keybindings
                                       links
                                       models
                                       presentation
                                       protocols
                                       providers
                                       selection
                                       state
                                       tools
                                       usage])}
