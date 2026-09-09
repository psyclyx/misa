(fn latest-prompt [messages]
  (let [message (. messages (length messages))]
    (assert (and message (= message.role :user))
            "command provider requires a final user message")
    (let [parts {}]
      (each [_ block (ipairs message.content)]
        (when (= block.type :text)
          (tset parts (+ (length parts) 1) block.text)))
      (let [prompt (table.concat parts "\n")]
        (assert (not= prompt "") "command prompt must be nonempty")
        prompt))))

(fn request [argv effect]
  "Append the user prompt to the configured process arguments."
  (assert (and (= (type argv) :table) (> (length argv) 0))
          "command argv must be a nonempty array")
  (each [_ argument (ipairs argv)]
    (assert (and (= (type argument) :string) (not= argument "")
                 (not (argument:find "\000" 1 true)))
            "command argv must contain nonempty strings without NUL"))
  (assert (and (= (type effect.id) :string) (not= effect.id ""))
          "command id must be a nonempty string")
  (let [direct {}]
    (for [i 1 (length argv)]
      (tset direct i (. argv i)))
    (tset direct (+ (length direct) 1) (latest-prompt effect.messages))
    {:argv direct
     :completion :provider/command-complete
     :id effect.id
     :type :provider/process}))

(fn complete [_ event]
  "Translate process completion into normalized stream events."
  (assert (and (= (type event.id) :string) (not= event.id ""))
          "command completion id must be nonempty")
  (let [fx [{:event {:id event.id :type :agent/stream-start} :type :dispatch}]]
    (if event.ok
        (do
          (tset fx (+ (length fx) 1)
                {:event {:delta {:text event.stdout :type :text}
                         :id event.id
                         :type :agent/stream-delta}
                 :type :dispatch})
          (tset fx (+ (length fx) 1)
                {:event {:id event.id :type :agent/stream-end} :type :dispatch}))
        (tset fx (+ (length fx) 1)
              {:event {:id event.id
                       :message (or (and (not= event.stderr "") event.stderr)
                                    (.. "command exited "
                                        (tostring event.status)))
                       :type :agent/stream-error}
               :type :dispatch}))
    {: fx}))

{:request request :complete complete}
