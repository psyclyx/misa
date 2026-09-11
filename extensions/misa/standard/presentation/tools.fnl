(local tools (require :misa.transcript.tools))

{:services {:tools.presentation tools.tools-presentation}
 :tool-presentations {:shell {:fields [:command]
                              :code :command
                              :language :sh
                              :numbered false
                              :result tools.shell-result}
                      :read_file {:subject :path
                                  :fields [:path]
                                  :result tools.read-result}
                      :list_directory {:subject :path :fields [:path]}
                      :write_file {:subject :path
                                   :fields [:path :content]
                                   :code :content}
                      :edit_file tools.edit-view
                      :web_search {:subject :query :fields [:query]}}
 :validators {:tool-presentations tools.validate-binding}}
