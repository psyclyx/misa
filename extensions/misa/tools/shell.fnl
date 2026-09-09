(local definitions (require :misa.definitions))

;; Shell tool policy over Zig's direct process capability.

(fn build [context]
  "Build the declarations for tool shell."
  (let [declarations []]
    (var config (or (and (= (type context.config) :table) context.config.tools)
                    nil))
    (set config (or (and (= (type config) :table) config.shell) nil))
    (set config (or (and (= (type config) :table) config) {}))
    (let [executable (or config.executable :sh)]
      (assert (and (= (type executable) :string) (not= executable ""))
              "config.tools.shell.executable must be nonempty")
      (table.insert declarations
                    (let [definition {:description "Run a shell command in Misa's working directory and return its captured output."
                                      :effect :tool.shell/run
                                      :input_schema {:additionalProperties false
                                                     :properties {:command {:description "Shell command to execute"
                                                                            :type :string}}
                                                     :required [:command]
                                                     :type :object}
                                      :name :shell}]
                      {:catalog :tools
                       :id (. definition :name)
                       :value definition}))
      (table.insert declarations
                    {:catalog :effects
                     :id :tool.shell/run
                     :value (fn [effect]
                              (let [arguments (assert (and (= (type effect.arguments)
                                                              :table)
                                                           effect.arguments)
                                                      "shell arguments must be an object")]
                                (assert (and (= (type arguments.command)
                                                :string)
                                             (not= arguments.command ""))
                                        "command must be nonempty")
                                {:argv [executable :-lc arguments.command]
                                 :completion :tool/shell-complete
                                 :id effect.tool_call_id
                                 :type :process/run}))})
      (table.insert declarations
                    {:catalog :events
                     :value {:event :tool/shell-complete
                             :handler (fn [_ event]
                                        (var text (or event.stdout ""))
                                        (when (and event.stderr
                                                   (not= event.stderr ""))
                                          (when (and (not= text "")
                                                     (not= (text:sub (- 1))
                                                           "\n"))
                                            (set text (.. text "\n")))
                                          (set text (.. text event.stderr)))
                                        (when (not event.ok)
                                          (let [messages {:Canceled "Command cancelled."
                                                          :StartupTimeout "The command did not start producing output before the timeout."
                                                          :IdleTimeout "The command stopped producing output and timed out."
                                                          :OverallTimeout "The command exceeded its time limit."
                                                          :FileNotFound "The shell executable could not be found. Check the configured shell path."
                                                          :AccessDenied "Permission denied when starting the shell."}
                                                headline (or (. messages
                                                                event.message)
                                                             (.. "Command exited with status "
                                                                 (tostring event.status)
                                                                 "."))]
                                            (set text
                                                 (.. headline
                                                     (if (= text "") ""
                                                         (.. "\n" text))))))
                                        (when (= text "")
                                          (set text
                                               (or (and event.ok
                                                        "command completed with no output")
                                                   (.. "command exited "
                                                       (tostring event.status)))))
                                        {:fx [{:event {:is_error (not event.ok)
                                                       : text
                                                       :tool_call_id event.id
                                                       :type :tool/result}
                                               :type :dispatch}]})}})
      (definitions.build :tool.shell declarations {}))))

{: build}
