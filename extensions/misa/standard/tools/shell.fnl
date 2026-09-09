(local shell (require :misa.tools.shell))
{:tools {:shell {:name :shell
                 :description "Run a shell command in Misa's working directory and return its captured output."
                 :effect :tool.shell/run
                 :input_schema {:type :object
                                :additionalProperties false
                                :properties {:command {:type :string
                                                       :description "Shell command to execute"}}
                                :required [:command]}}}
 :effects {:tool.shell/run (fn [effect cofx]
                             (shell.process-effect (shell.executable (or (and cofx.config.tools
                                                                              cofx.config.tools.shell)
                                                                         {}))
                                                   effect))}
 :events {:tool.shell/tool/shell-complete {:event :tool/shell-complete
                                           :priority 13000
                                           :handler shell.completed}}}
