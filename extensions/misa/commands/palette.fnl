;; Command palette composed from registered commands and canonical recent
;; invocations. Argument completion is an ordinary narrowing source.

(fn command-items [db]
  "List registered commands and recent invocations."
  (let [result {}
        seen {}]
    (each [_ command (ipairs (misa.commands.all))]
      (let [narrow (when (or command.completion command.complete)
                     {:context {:command command.name}
                      :source :command-arguments})]
        (tset result (+ (length result) 1)
              {:display {:description command.description :label command.name}
               :id command.name
               :invocation (or (and (not narrow) command.name) nil)
               : narrow
               :path command.name
               :preview {:description command.description :title command.name}
               :search [command.name command.description]
               :value command.name})
        (tset seen command.name true)))
    (each [_ entry (ipairs (misa.commands.recent db))]
      (when (not (. seen entry.canonical))
        (tset seen entry.canonical true)
        (tset result (+ (length result) 1)
              {:display {:description "recent invocation"
                         :label entry.canonical}
               :id entry.canonical
               :invocation entry.canonical
               :path entry.canonical
               :preview {:kind :recent :title entry.canonical}
               :search [entry.command entry.arguments]
               :value entry.canonical})))
    result))

(fn session [db query]
  "Create a choice session for the unified picker."
  (let [spec (misa.choices.source :omnipicker nil db)]
    (set spec.query (or query ""))
    (misa.choices.session spec db)))

(fn open-picker [db]
  "Open the command palette with a fresh correlation token."
  (let [sequence (+ (or db.omnipicker_sequence 0) 1)
        token (.. "omnipicker:" sequence)]
    {:patch {:omnipicker_sequence sequence}
     :fx [{:event {:completion :omnipicker/selected
                   :id :omnipicker
                   :nested (not= db.picker nil)
                   :session (misa.picker.session db "")
                   :title :Commands
                   : token
                   :type :picker/open}
           :type :dispatch}]}))

(fn select-command [db event]
  "Dispatch the invocation selected from the command palette."
  (if (not= event.picker :omnipicker) nil
      (if event.cancelled
          {:fx [{:type :terminal/read}]}
          (let [invocation (assert (misa.commands.invocation (or event.invocation
                                                                 (tostring event.value)))
                                   "invalid command palette invocation")]
            {:fx [{:event invocation :type :dispatch}]}))))

{:command-items command-items
 :open-picker open-picker
 :select-command select-command
 :session session}
