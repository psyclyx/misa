(local definitions (require :misa.definitions))

;; File tools are Lua policy over small native file effects. Paths are used as
;; supplied; filesystem sandboxing belongs to the environment running Misa.

(fn schema [properties required]
  {:additionalProperties false : properties : required :type :object})

(fn build []
  "Build the declarations for tool files."
  (let [declarations []]
    (table.insert declarations
                  (let [definition {:description "Read a UTF-8 text file as a snapshot header and LINE#HASH|text rows. Copy the snapshot and LINE#HASH anchors into edit_file. Reads return at most 2000 lines; use start_line and max_lines for another page. Paths may be absolute or relative to Misa's working directory."
                                    :effect :tool.files/read
                                    :input_schema (schema {:path {:description "File path"
                                                                  :type :string}
                                                           :start_line {:description "First line to return (1-based, defaults to 1)"
                                                                        :type :integer
                                                                        :minimum 1
                                                                        :maximum 1048576}
                                                           :max_lines {:description "Maximum lines to return (defaults to 2000)"
                                                                       :type :integer
                                                                       :minimum 1
                                                                       :maximum 2000}}
                                                          [:path])
                                    :name :read_file}]
                    {:catalog :tools
                     :id (. definition :name)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:description "List one directory. Directory names have a trailing slash."
                                    :effect :tool.files/list
                                    :input_schema (schema {:path {:description "Directory path"
                                                                  :type :string}}
                                                          [:path])
                                    :name :list_directory}]
                    {:catalog :tools
                     :id (. definition :name)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:description "Create or replace a UTF-8 text file with the exact supplied content."
                                    :effect :tool.files/write
                                    :input_schema (schema {:content {:description "Complete new file content"
                                                                     :type :string}
                                                           :path {:description "File path"
                                                                  :type :string}}
                                                          [:path :content])
                                    :name :write_file}]
                    {:catalog :tools
                     :id (. definition :name)
                     :value definition}))
    (table.insert declarations
                  (let [definition {:description "Edit a UTF-8 file using anchors from read_file: supply snapshot, start LINE#HASH, optional inclusive end (defaults to start), new_text, and position replace (default), before, or after. Empty new_text deletes a replacement range. Line anchors and the whole snapshot must still match; re-read after any edit or stale-snapshot error. new_text contains plain replacement lines, without read prefixes; a final newline is optional."
                                    :effect :tool.files/edit
                                    :input_schema (schema {:new_text {:description "Replacement text"
                                                                      :type :string}
                                                           :snapshot {:description "Snapshot tag from the latest read_file header"
                                                                      :type :string}
                                                           :start {:description "First LINE#HASH anchor from read_file"
                                                                   :type :string}
                                                           :end {:description "Last inclusive LINE#HASH anchor (defaults to start)"
                                                                 :type :string}
                                                           :position {:description "replace (default), before, or after; insertions require one anchor"
                                                                      :enum [:replace
                                                                             :before
                                                                             :after]
                                                                      :type :string}
                                                           :path {:description "File path"
                                                                  :type :string}}
                                                          [:path
                                                           :snapshot
                                                           :start
                                                           :new_text])
                                    :name :edit_file}]
                    {:catalog :tools
                     :id (. definition :name)
                     :value definition}))

    (fn argument [effect name]
      (let [arguments (assert (and (= (type effect.arguments) :table)
                                   effect.arguments)
                              "file tool arguments must be an object")
            value (. arguments name)]
        (assert (= (type value) :string) (.. name " must be a string"))
        value))

    (table.insert declarations
                  {:catalog :effects
                   :id :tool.files/read
                   :value (fn [effect]
                            (let [path (argument effect :path)]
                              (each [name maximum (pairs {:start_line 1048576
                                                          :max_lines 2000})]
                                (let [value (. effect.arguments name)]
                                  (when (not= value nil)
                                    (assert (and (= (type value) :number)
                                                 (>= value 1) (<= value maximum)
                                                 (= value (math.floor value)))
                                            (.. name
                                                " must be a positive integer within its limit")))))
                              {:completion :tool/files-complete
                               :id effect.tool_call_id
                               : path
                               :start_line effect.arguments.start_line
                               :max_lines effect.arguments.max_lines
                               :anchored true
                               :type :file/read}))})
    (table.insert declarations
                  {:catalog :effects
                   :id :tool.files/list
                   :value (fn [effect]
                            {:completion :tool/files-complete
                             :id effect.tool_call_id
                             :path (argument effect :path)
                             :type :file/list})})
    (table.insert declarations
                  {:catalog :effects
                   :id :tool.files/write
                   :value (fn [effect]
                            {:completion :tool/files-complete
                             :content (argument effect :content)
                             :id effect.tool_call_id
                             :path (argument effect :path)
                             :type :file/write})})
    (table.insert declarations
                  {:catalog :effects
                   :id :tool.files/edit
                   :value (fn [effect]
                            (let [path (argument effect :path)
                                  content (argument effect :new_text)
                                  args effect.arguments]
                              (assert (= args.old_text nil)
                                      "old_text is no longer supported. Read the file and use snapshot/line anchors.")
                              (when (not= args.end nil) (argument effect :end))
                              (when (not= args.position nil)
                                (argument effect :position))
                              {:completion :tool/files-complete
                               :id effect.tool_call_id
                               : path
                               : content
                               :snapshot (argument effect :snapshot)
                               :start (argument effect :start)
                               :end args.end
                               :position args.position
                               :type :file/edit_lines}))})
    (let [errors {:IsDir "This path is a directory. Use list_directory to view its contents."
                  :NotDir "This path is not a directory. Use read_file to read a file."
                  :FileNotFound "No file or directory exists at this path. Check the path and try again."
                  :AccessDenied "Permission denied. Choose a path you have permission to access."
                  :PermissionDenied "Permission denied. Choose a path you have permission to access."
                  :ReadOnlyFileSystem "This file is on a read-only filesystem and cannot be changed."
                  :NoSpaceLeft "There is not enough free space to write this file."
                  :DiskQuota "The storage quota is full. Free space before writing this file."
                  :NameTooLong "This path is too long. Use a shorter path."
                  :SymLinkLoop "This path contains a loop of symbolic links. Use the target's direct path."
                  :FileTooBig "This file is too large for the file tool. Read a smaller file or use shell to inspect a portion."
                  :StreamTooLong "The result is too large for the file tool. Narrow the path or use shell to inspect a portion."
                  :TooManyEntries "This directory has too many entries. Use shell to filter its contents."
                  :NotUtf8Text "This file contains binary or non-UTF-8 data. The file tool requires UTF-8 text."
                  :PatternNotFound "The text to replace was not found. Read the file again and use its current text."
                  :PatternNotUnique "The text to replace occurs more than once. Include more context or use line anchors."
                  :InvalidLineAnchor "The line anchor is invalid. Copy a LINE#HASH anchor from read_file."
                  :StaleLineAnchor "The line has changed. Read the file again before editing."
                  :StaleSnapshotReadFileAgain "The file has changed. Read it again before editing."
                  :LineOutOfRange "The requested line is past the end of the file. Read the file again to check its lines."
                  :ReversedLineRange "The end line comes before the start line. Put the anchors in file order."
                  :InsertionRequiresSingleAnchor "Insertion requires one line anchor. Use the same start and end anchor."
                  :EffectTooLarge "The proposed file content is too large. Make a smaller edit."
                  :EditDidNotChangeFile "The replacement is identical to the current text. No change was made."}]
      (table.insert declarations
                    {:catalog :events
                     :value {:event :tool/files-complete
                             :handler (fn [_ event]
                                        {:fx [{:event {:is_error (not event.ok)
                                                       :text (or (and event.ok
                                                                      event.text)
                                                                 (and (. errors
                                                                         event.message)
                                                                      (.. (. errors
                                                                             event.message)
                                                                          "\nDetails: "
                                                                          event.message))
                                                                 event.message)
                                                       :tool_call_id event.id
                                                       :type :tool/result}
                                               :type :dispatch}]})}})
      (definitions.build :tool.files declarations {}))))

{: build}
