(local definitions (require :misa.definitions))

;; Presentation bindings belong here, independently of tool definitions and
;; independently of the generic components that display their values.
(fn display [value]
  (if (= (type value) :string) value (= (type value) :table)
      (let [keys []]
        (each [key (pairs value)] (table.insert keys key))
        (table.sort keys (fn [a b] (< (tostring a) (tostring b))))
        (.. "{" (table.concat (icollect [_ key (ipairs keys)]
                                (.. (tostring key) ": " (display (. value key))))
                              ", ") "}")) (tostring value)))

(fn descriptor [role model] {:role (.. :content. role) : model})
(fn subject [model binding]
  (let [value (and binding binding.subject model.arguments
                   (. model.arguments binding.subject))]
    (when (not= value nil)
      (let [text (display value)]
        (when (not (text:find "\n" 1 true)) {:type :text :value text})))))

(fn arguments [model binding heading]
  (if (not= (type model.arguments) :table)
      (if binding [] [(descriptor :text
                                  {:text (or model.argument_text
                                             (table.concat (or model.argument_chunks
                                                               [])))
                                   :style :tool})])
      (let [args model.arguments
            fields []
            keys (or (and binding binding.fields)
                     (let [all []]
                       (each [key (pairs args)] (table.insert all key))
                       (table.sort all (fn [a b] (< (tostring a) (tostring b))))
                       all))]
        (each [_ key (ipairs keys)]
          (when (and (not= (. args key) nil)
                     (not= key (and binding binding.code))
                     (not= key (and heading binding.subject)))
            (table.insert fields
                          {:label (tostring key) :value (display (. args key))})))
        (let [sections []]
          (when (> (length fields) 0)
            (table.insert sections (descriptor :fields {: fields :style :tool})))
          (when (and binding binding.code (not= (. args binding.code) nil)
                     (not (and heading (= binding.subject binding.code))))
            (table.insert sections
                          (descriptor :code
                                      {:text (display (. args binding.code))
                                       :source false
                                       :style :tool
                                       :language binding.language
                                       :numbered binding.numbered
                                       :syntax model.syntax})))
          sections))))

(fn result [model]
  (let [value (or model.result model.text)]
    (when (not= value nil)
      (let [text (tostring value)]
        (descriptor (if (or (text:match "^diff %-%-git ") (text:match "^@@ %-")
                            (text:find "\n@@ %-"))
                        :diff
                        :text) {: text :style :tool})))))

(fn file-result [model]
  (let [text (or model.result model.text "")]
    (when (text:match "^snapshot %x+\n")
      (let [rows []]
        (var offset 0)
        (each [line (: (.. text "\n") :gmatch "(.-)\n")]
          (let [(number header value) (line:match "^(%d+)(#%x+|)(.*)$")]
            (if number
                (table.insert rows
                              {:number (tonumber number)
                               :text value
                               :source_start (+ offset (length number)
                                                (length header))})
                (and (not (line:match "^snapshot %x+$")) (not= line ""))
                (table.insert rows {:text line :source_start offset}))
            (set offset (+ offset (length line) 1))))
        (descriptor :lines {: rows :style :tool})))))

(fn edit-result [model]
  (let [text (or model.result "")]
    (when (and (not model.is_error) (text:match "^@@ %-"))
      (let [finish (text:find "\n\nsnapshot " 1 true)]
        (descriptor :diff {:text (if finish (text:sub 1 (- finish 1)) text)
                           :preview_limit 12
                           :style :tool})))))

(fn edit-view [model]
  (let [diff (edit-result model)
        binding {:subject :path
                 :fields [:path :new_text]
                 :code :new_text
                 :numbered false}
        heading (subject model binding)]
    {:subject heading
     :prefer_result (not= diff nil)
     :arguments (if diff [] (arguments model binding heading))
     :result (or diff (and (not model.is_error) (file-result model))
                 (result model))}))

(fn shell-result [model]
  (let [value (or model.result model.text)]
    (when (not= value nil)
      (descriptor :code
                  {:text (tostring value)
                   :numbered false
                   :style :tool
                   :preview_tail true
                   :missing_newline "\\ No newline at end of output"}))))

(fn read-result [model]
  (if model.is_error (result model)
      (or (file-result model)
          (when (or model.result model.text)
            (descriptor :code {:text (or model.result model.text) :style :tool})))))

(fn tools-presentation [roles model]
  "Describe a tool call using its configured presentation adapter."
  (let [name (or model.name "")
        selected (. roles name)
        binding (. (misa.catalog :tool-presentations) (or selected name))]
    (when selected
      (assert binding (.. "unknown tool presentation: " selected)))
    (let [heading (when (not= (type binding) :function)
                    (subject model binding))]
      (if (= (type binding) :function) (binding model)
          {:subject heading
           :arguments (arguments model binding heading)
           :result (or (and binding binding.result (binding.result model))
                       (result model))}))))

(fn build [context]
  "Build the declarations for tool presentations."
  (let [roles (or (. (or (. (or context.config {}) :tool_presentations) {})
                     :roles) {})
        fx [{:catalog :services
             :id :tools.presentation
             :value (fn [model] (tools-presentation roles model))}]]
    (each [name binding (pairs {:shell {:fields [:command]
                                        :code :command
                                        :language :sh
                                        :numbered false
                                        :result shell-result}
                                :read_file {:subject :path
                                            :fields [:path]
                                            :result read-result}
                                :list_directory {:subject :path
                                                 :fields [:path]}
                                :write_file {:subject :path
                                             :fields [:path :content]
                                             :code :content}
                                :edit_file edit-view})]
      (table.insert fx {:catalog :tool-presentations :id name :value binding}))
    (definitions.build :tool_presentations
      fx
      {:validators {:tool-presentations (fn [_ binding]
                                          (assert (or (= (type binding) :table)
                                                      (= (type binding)
                                                         :function))
                                                  "tool presentation requires an adapter or binding"))}})))

{:build build}
