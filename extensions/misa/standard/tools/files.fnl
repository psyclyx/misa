(local implementation (require :misa.tools.files))

{:tools {:read_file {:description "Read a UTF-8 text file as a snapshot header and LINE#HASH|text rows. Copy the snapshot and LINE#HASH anchors into edit_file. Reads return at most 2000 lines; use start_line and max_lines for another page. Paths may be absolute or relative to Misa's working directory."
                     :effect :tool.files/read
                     :input_schema (implementation.schema {:path {:description "File path"
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
                     :name :read_file}
         :list_directory {:description "List one directory. Directory names have a trailing slash."
                          :effect :tool.files/list
                          :input_schema (implementation.schema {:path {:description "Directory path"
                                                                       :type :string}}
                                                               [:path])
                          :name :list_directory}
         :write_file {:description "Create or replace a UTF-8 text file with the exact supplied content."
                      :effect :tool.files/write
                      :input_schema (implementation.schema {:content {:description "Complete new file content"
                                                                      :type :string}
                                                            :path {:description "File path"
                                                                   :type :string}}
                                                           [:path :content])
                      :name :write_file}
         :edit_file {:description "Edit a UTF-8 file using anchors from read_file: supply snapshot, start LINE#HASH, optional inclusive end (defaults to start), new_text, and position replace (default), before, or after. Empty new_text deletes a replacement range. Line anchors and the whole snapshot must still match; re-read after any edit or stale-snapshot error. new_text contains plain replacement lines, without read prefixes; a final newline is optional."
                     :effect :tool.files/edit
                     :input_schema (implementation.schema {:new_text {:description "Replacement text"
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
                     :name :edit_file}}
 :effects {:tool.files/read implementation.read-effect
           :tool.files/list implementation.list-effect
           :tool.files/write implementation.write-effect
           :tool.files/edit implementation.edit-effect}
 :events {:tool.files/tool/files-complete {:event :tool/files-complete
                                           :handler implementation.completed
                                           :priority 12000}}}
