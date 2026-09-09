(local errors
       {:IsDir "This path is a directory. Use list_directory to view its contents."
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
        :EditDidNotChangeFile "The replacement is identical to the current text. No change was made."})

;; File tools are Lua policy over small native file effects. Paths are used as
;; supplied; filesystem sandboxing belongs to the environment running Misa.

(fn schema [properties required]
  "Describe a tool input object and its required fields."
  {:additionalProperties false : properties : required :type :object})

(fn argument [effect name]
  (let [arguments (assert (and (= (type effect.arguments) :table)
                               effect.arguments)
                          "file tool arguments must be an object")
        value (. arguments name)]
    (assert (= (type value) :string) (.. name " must be a string"))
    value))

(fn read-effect [effect]
  "Validate a file read and describe its native effect."
  (let [path (argument effect :path)]
    (each [name maximum (pairs {:start_line 1048576 :max_lines 2000})]
      (let [value (. effect.arguments name)]
        (when (not= value nil)
          (assert (and (= (type value) :number) (>= value 1) (<= value maximum)
                       (= value (math.floor value)))
                  (.. name " must be a positive integer within its limit")))))
    {:completion :tool/files-complete
     :id effect.tool_call_id
     : path
     :start_line effect.arguments.start_line
     :max_lines effect.arguments.max_lines
     :anchored true
     :type :file/read}))

(fn list-effect [effect]
  "Describe a directory listing effect."
  {:completion :tool/files-complete
   :id effect.tool_call_id
   :path (argument effect :path)
   :type :file/list})

(fn write-effect [effect]
  "Describe a file replacement effect."
  {:completion :tool/files-complete
   :content (argument effect :content)
   :id effect.tool_call_id
   :path (argument effect :path)
   :type :file/write})

(fn edit-effect [effect]
  "Validate an anchored edit and describe its native effect."
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
     :type :file/edit_lines}))

(fn completed [_ event]
  "Translate file completion into a tool result."
  {:fx [{:event {:is_error (not event.ok)
                 :text (or (and event.ok event.text)
                           (and (. errors event.message)
                                (.. (. errors event.message) "\nDetails: "
                                    event.message))
                           event.message)
                 :tool_call_id event.id
                 :type :tool/result}
         :type :dispatch}]})

{:read-effect read-effect
 :edit-effect edit-effect
 :write-effect write-effect
 :schema schema
 :list-effect list-effect
 :completed completed}
