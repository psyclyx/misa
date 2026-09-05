;; File tools are Lua policy over small native file effects. Paths are used as

;; supplied; filesystem sandboxing belongs to the environment running Misa.

(fn schema [properties required]
  {:additionalProperties false : properties : required :type :object})

{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/tool
                         :value {:description "Read a UTF-8 text file. Paths may be absolute or relative to Misa's working directory."
                                 :effect :tool.files/read
                                 :input_schema (schema {:path {:description "File path"
                                                               :type :string}}
                                                       [:path])
                                 :name :read_file}})
          (table.insert setup-fx
                        {:type :register/tool
                         :value {:description "List one directory. Directory names have a trailing slash."
                                 :effect :tool.files/list
                                 :input_schema (schema {:path {:description "Directory path"
                                                               :type :string}}
                                                       [:path])
                                 :name :list_directory}})
          (table.insert setup-fx
                        {:type :register/tool
                         :value {:description "Create or replace a UTF-8 text file with the exact supplied content."
                                 :effect :tool.files/write
                                 :input_schema (schema {:content {:description "Complete new file content"
                                                                  :type :string}
                                                        :path {:description "File path"
                                                               :type :string}}
                                                       [:path :content])
                                 :name :write_file}})
          (table.insert setup-fx
                        {:type :register/tool
                         :value {:description "Replace one exact, uniquely occurring string in a UTF-8 text file."
                                 :effect :tool.files/edit
                                 :input_schema (schema {:new_text {:description "Replacement text"
                                                                   :type :string}
                                                        :old_text {:description "Exact text to replace; it must occur once"
                                                                   :type :string}
                                                        :path {:description "File path"
                                                               :type :string}}
                                                       [:path
                                                        :old_text
                                                        :new_text])
                                 :name :edit_file}})

          (fn argument [effect name]
            (local arguments
                   (assert (and (= (type effect.arguments) :table)
                                effect.arguments)
                           "file tool arguments must be an object"))
            (local value (. arguments name))
            (assert (= (type value) :string) (.. name " must be a string"))
            value)

          (table.insert setup-fx
                        {:type :register/fx
                         :name :tool.files/read
                         :handler (fn [effect]
                                    {:completion :tool/files-complete
                                     :id effect.tool_call_id
                                     :path (argument effect :path)
                                     :type :file/read})})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :tool.files/list
                         :handler (fn [effect]
                                    {:completion :tool/files-complete
                                     :id effect.tool_call_id
                                     :path (argument effect :path)
                                     :type :file/list})})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :tool.files/write
                         :handler (fn [effect]
                                    {:completion :tool/files-complete
                                     :content (argument effect :content)
                                     :id effect.tool_call_id
                                     :path (argument effect :path)
                                     :type :file/write})})
          (table.insert setup-fx
                        {:type :register/fx
                         :name :tool.files/edit
                         :handler (fn [effect]
                                    {:completion :tool/files-complete
                                     :content (argument effect :old_text)
                                     :id effect.tool_call_id
                                     :path (argument effect :path)
                                     :replacement (argument effect :new_text)
                                     :type :file/edit})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :tool/files-complete
                         :handler (fn [_ event]
                                    {:fx [{:event {:is_error (not event.ok)
                                                   :text (or (and event.ok
                                                                  event.text)
                                                             event.message)
                                                   :tool_call_id event.id
                                                   :type :tool/result}
                                           :type :dispatch}]})})
          {:fx setup-fx})}
