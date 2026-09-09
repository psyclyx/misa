(local definitions (require :misa.definitions))

(local errors
       {:Canceled "Command cancelled."
        :StartupTimeout "The command did not start producing output before the timeout."
        :IdleTimeout "The command stopped producing output and timed out."
        :OverallTimeout "The command exceeded its time limit."
        :FileNotFound "The shell executable could not be found. Check the configured shell path."
        :AccessDenied "Permission denied when starting the shell."})

(fn output-text [event]
  "Join captured streams and explain unsuccessful process completion."
  (let [stdout (or event.stdout "")
        stderr (or event.stderr "")
        separator (if (and (not= stdout "") (not= stderr "")
                           (not= (stdout:sub -1) "\n"))
                      "\n"
                      "")
        output (.. stdout separator stderr)]
    (if (not event.ok)
        (.. (or (. errors event.message)
                (.. "Command exited with status " (tostring event.status) "."))
            (if (= output "") "" (.. "\n" output)))
        (= output "")
        "command completed with no output"
        output)))

(fn process-effect [executable effect]
  "Validate a shell invocation and describe its process effect."
  (let [arguments (assert (and (= (type effect.arguments) :table)
                               effect.arguments)
                          "shell arguments must be an object")]
    (assert (and (= (type arguments.command) :string)
                 (not= arguments.command ""))
            "command must be nonempty")
    {:argv [executable :-lc arguments.command]
     :completion :tool/shell-complete
     :id effect.tool_call_id
     :type :process/run}))

(fn completed [_ event]
  {:fx [{:type :dispatch
         :event {:type :tool/result
                 :tool_call_id event.id
                 :is_error (not event.ok)
                 :text (output-text event)}}]})

(fn build [context]
  "Declare the shell tool with its configured executable."
  (let [tools (and (= (type context.config) :table) context.config.tools)
        raw (and (= (type tools) :table) tools.shell)
        config (if (= (type raw) :table) raw {})
        executable (or config.executable :sh)]
    (assert (and (= (type executable) :string) (not= executable ""))
            "config.tools.shell.executable must be nonempty")
    (definitions.build :tool.shell
      [{:catalog :tools
        :id :shell
        :value {:name :shell
                :description "Run a shell command in Misa's working directory and return its captured output."
                :effect :tool.shell/run
                :input_schema {:type :object
                               :additionalProperties false
                               :properties {:command {:description "Shell command to execute"
                                                      :type :string}}
                               :required [:command]}}}
       {:catalog :effects
        :id :tool.shell/run
        :value (fn [effect] (process-effect executable effect))}
       {:catalog :events
        :value {:event :tool/shell-complete :handler completed}}])))

{: build : output-text : process-effect}
