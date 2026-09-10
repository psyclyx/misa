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
  "Translate captured process completion into a tool result."
  {:fx [{:type :dispatch
         :event {:type :tool/result
                 :tool_call_id event.id
                 :is_error (not event.ok)
                 :text (output-text event)}}]})

(fn executable [config]
  "Validate and return the configured shell executable."
  (let [name (or config.executable :sh)]
    (assert (and (= (type name) :string) (not= name ""))
            "config.tools.shell.executable must be nonempty")
    name))

{: output-text : process-effect : completed : executable}
