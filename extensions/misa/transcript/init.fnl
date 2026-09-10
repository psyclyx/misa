(local {: viewport} (require :misa.transcript.viewport))

(fn noninteractive-commit [db role model cofx markdown]
  "Render a noninteractive message as terminal commit effects."
  (if (or cofx.terminal.interactive
          (not (and misa.components misa.components.render)))
      {}
      (let [rendered (misa.components.render db role model
                                             {:columns cofx.terminal.columns
                                              :interactive false
                                              : markdown})]
        (if (= (length (or rendered.lines {})) 0) {}
            [{:lines rendered.lines :type :view/commit}]))))

(fn messages-detail-indicator-value [inputs]
  "Describe the selected transcript detail level."
  {:type :text :value (if (. inputs 1) :verbose :summary)})

(fn terminal-input [scroll-inputs db event cofx]
  "Resolve transcript navigation and detail actions from an input event."
  (when (and (not db.picker) (not db.dialog) misa.keybindings
             misa.keybindings.action)
    (let [action (misa.keybindings.action :global event)
          scroll (or (. scroll-inputs event.kind) (. scroll-inputs action))]
      (if scroll {:type :messages/scroll :delta (scroll cofx)}
          (= action :toggle_verbose) {:type :messages/toggle-verbose}))))

(fn transcript-window [db context room]
  "Return the current visible transcript window."
  (. (viewport db context room) :lines))

(fn runtime-dispatch-limit [_ event]
  "Present an exhausted event budget as a harness error."
  {:fx [{:type :dispatch
         :event {:type :transcript/harness :level :error :text event.text}}]})

(fn projection-inputs [db]
  "Select the state that invalidates transcript presentation."
  (let [state (assert db.messages "message state is not initialized")]
    {:blocks state.blocks
     :responses state.responses
     :by_response state.by_response
     :verbose state.verbose
     :syntax db.syntax
     :selection db.selection
     :costs db.costs
     :components db.components
     :themes db.themes
     :hover_action db.hover_action
     :hover_link db.hover_link
     :choice_pending (and misa.choices misa.choices.pending
                          (misa.choices.pending db))}))

(fn validate-presentation [_ handler]
  "Require a callable transcript presentation adapter."
  (assert (= (type handler) :function) "presentation must be a function"))

(fn validate-delta [_ handler]
  "Require a callable transcript delta handler."
  (assert (= (type handler) :function) "delta handler must be a function"))

{: messages-detail-indicator-value
 : noninteractive-commit
 : projection-inputs
 : runtime-dispatch-limit
 : terminal-input
 : transcript-window
 : validate-delta
 : validate-presentation}
