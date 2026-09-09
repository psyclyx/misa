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
  "Describe an edit using its diff and argument content."
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
  "Describe shell output as a code block with trailing-line semantics."
  (let [value (or model.result model.text)]
    (when (not= value nil)
      (descriptor :code
                  {:text (tostring value)
                   :numbered false
                   :style :tool
                   :preview_tail true
                   :missing_newline "\\ No newline at end of output"}))))

(fn read-result [model]
  "Describe file output with consistent source coordinates."
  (if model.is_error (result model)
      (or (file-result model)
          (when (or model.result model.text)
            (descriptor :code {:text (or model.result model.text) :style :tool})))))

(fn tools-presentation [model]
  "Describe a tool call using its named presentation adapter."
  (let [binding (. (misa.catalog :tool-presentations) (or model.name ""))
        heading (when (not= (type binding) :function) (subject model binding))]
    (if (= (type binding) :function) (binding model)
        {:subject heading
         :arguments (arguments model binding heading)
         :result (or (and binding binding.result (binding.result model))
                     (result model))})))

(fn validate-binding [_ binding]
  "Require a presentation adapter or declarative tool binding."
  (assert (or (= (type binding) :table) (= (type binding) :function))
          "tool presentation requires an adapter or binding"))

{: edit-view
 : read-result
 : shell-result
 : tools-presentation
 : validate-binding}
