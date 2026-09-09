{:setup (fn []
          (local setup-fx [])
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:event {:text "# Heading with **emphasis**, *italics*, ~~gone~~, `code`, and [docs](https://example.test)"
                                                   :type :transcript/user}
                                           :type :dispatch}
                                          {:event {:content [{:text :private
                                                              :type :thinking}]
                                                   :request_id :thought
                                                   :type :transcript/assistant}
                                           :type :dispatch}
                                          {:event {:arguments {:value :detail}
                                                   :id :call
                                                   :name :demo
                                                   :type :transcript/tool-call}
                                           :type :dispatch}
                                          {:event {:id :call
                                                   :text :result
                                                   :type :transcript/tool-result}
                                           :type :dispatch}]})})
          (table.insert setup-fx
                        {:type :register/event
                         :name :transcript/tool-result
                         :handler (fn [db]
                                    (local lines
                                           (misa.transcript_projection db
                                                                       {:columns 32
                                                                        :interactive true}))
                                    (assert (= (. lines 1 :spans 1 :text) "┃ ")
                                            "user content should begin directly with its rail")
                                    (var (bold italic strike linked rails)
                                         (values false false false false {}))

                                    (fn color-key [c]
                                      (or (and (= (type c) :table)
                                               (.. c.r "," c.g "," c.b))
                                          c))

                                    (each [_ line (ipairs lines)]
                                      (each [_ span (ipairs line.spans)]
                                        (when (and (= span.style.bold true)
                                                   (span.text:find :emphasis 1
                                                                   true))
                                          (set bold true))
                                        (when (= span.style.italic true)
                                          (set italic true))
                                        (when (= span.style.strikethrough true)
                                          (set strike true))
                                        (when (= span.link
                                                 "https://example.test")
                                          (set linked true))
                                        (when (= span.text "┃ ")
                                          (tset rails
                                                (color-key span.style.foreground)
                                                true))))
                                    (assert (and (and (and bold italic) strike)
                                                 linked)
                                            "composed Markdown styles or links are missing")
                                    (each [_ role (ipairs [:user
                                                           :thinking
                                                           :tool])]
                                      (assert (. rails
                                                 (color-key (. (misa.theme_style db
                                                                                 (.. :rail.
                                                                                     role))
                                                               :foreground)))
                                              "semantic message rail lost its theme color"))
                                    (var sections 0)
                                    (each [_ block (ipairs db.messages.blocks)]
                                      (when (= block.kind :tool_call)
                                        (set sections (+ sections 1))
                                        (assert (and (= block.result :result)
                                                     (= block.status :success))
                                                "tool result did not update its call section")))
                                    (assert (and (= sections 1)
                                                 (= (length db.messages.blocks)
                                                    3))
                                            "matching tool result created an unrelated transcript block")
                                    {:fx [{:lines [{:spans [{:text :semantic}]}]
                                           :type :view/commit}
                                          {:type :app/quit}]})})
          nil
          {:fx setup-fx})}
