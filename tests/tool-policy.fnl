(local output io.write)
(local fennel (require :fennel))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local shell (require :misa.tools.shell))
(local files (require :misa.tools.files))
(local keys (require :misa.keybindings))

(fn fails? [f expected]
  (let [(ok message) (pcall f)]
    (assert (and (not ok) (: (tostring message) :find expected 1 true)))))

(assert (= (shell.output-text {:ok true :stdout "one\n" :stderr "two\n"})
           "one\ntwo\n"))

(assert (= (shell.output-text {:ok true :stdout "one" :stderr "two"})
           "one\ntwo"))

(assert (= (shell.output-text {:ok true}) "command completed with no output"))
(assert (= (shell.output-text {:ok false :status 3 :stdout "detail\n"})
           "Command exited with status 3.\ndetail\n"))

(assert (= (shell.output-text {:ok false :message :Canceled})
           "Command cancelled."))

(let [effect (shell.process-effect :bash
                                   {:tool_call_id :a
                                    :arguments {:command "echo hello"}})]
  (assert (= effect.type :process/run))
  (assert (= (. effect.argv 1) :bash))
  (assert (= (. effect.argv 3) "echo hello")))

(fails? (fn []
          (shell.process-effect :sh {:arguments {:command ""}}))
        "command must be nonempty")

(let [request {:tool_call_id :read
               :arguments {:path :file :start_line 10 :max_lines 2}}
      effect (files.read-effect request)]
  (assert (= effect.start_line 10))
  (assert (= effect.max_lines 2))
  (assert effect.anchored)
  (assert (= request.arguments.path :file)))

(each [_ value (ipairs [0 -1 1.5 2001])]
  (fails? (fn []
            (files.read-effect {:arguments {:path :file :max_lines value}}))
          "positive integer"))

(fails? (fn []
          (files.edit-effect {:arguments {:path :file
                                          :new_text ""
                                          :old_text "old"}}))
        "old_text is no longer supported")

(let [effect (files.edit-effect {:tool_call_id :edit
                                 :arguments {:path :file
                                             :new_text ""
                                             :snapshot :tag
                                             :start :1#abcd}})]
  (assert (= effect.content ""))
  (assert (= effect.type :file/edit_lines)))

(let [bindings [{:context :editor :action :submit :default [:enter]}]
      event {:kind :key :key :enter}]
  (assert (= (keys.resolve-action {} bindings :editor event) :submit))
  (assert (= (keys.resolve-action {:editor {:submit []}} bindings :editor event)
             nil))
  (assert (= (keys.resolve-action {:editor {:submit :x}} bindings :editor
                                  {:kind :text :text :x}) :submit))
  (fails? (fn []
            (keys.resolve-action {}
                                 [{:context :editor
                                   :action :a
                                   :default [:enter]}
                                  {:context :editor
                                   :action :b
                                   :default [:enter]}]
                                 :editor event))
          "ambiguous keybinding"))

(output "tool and keybinding policy contracts passed\n")
