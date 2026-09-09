(local definitions (require :misa.definitions))

;; External links use the ordinary process transport with a configurable opener.
(fn link-open [command db event]
  (assert (and (= (type event.url) :string)
               (event.url:match "^https?://[^%s]+$")
               (not (event.url:find "\000" 1 true)))
          "link URL must use HTTP or HTTPS")
  (let [sequence (+ (or db.link_sequence 0) 1)
        argv (misa.snapshot command)]
    (table.insert argv event.url)
    {:patch {:link_sequence sequence}
     :fx [{:type :process/run
           :id (.. "link:" sequence)
           :completion :link/completed
           :argv argv}
          {:type :terminal/read}]}))

(fn link-completed [_ event]
  (when (not event.ok)
    {:fx [{:type :dispatch
           :event {:type :transcript/harness
                   :level :error
                   :text "Could not open the external link; configure links.command for your browser."}}
          {:type :terminal/read}]}))

(fn build [context]
  "Build the declarations for links."
  (let [config (or context.config.links {})
        command (or config.command ["xdg-open"])]
    (assert (and (= (type command) :table) (> (length command) 0))
            "links.command must be a nonempty argv array")
    (each [_ argument (ipairs command)]
      (assert (and (= (type argument) :string) (not= argument "")
                   (not (argument:find "\000" 1 true)))
              "invalid link opener argument"))
    (definitions.build :links
      [{:catalog :events
        :value {:event :link/open
                :handler (fn [db event] (link-open command db event))}}
       {:catalog :events
        :value {:event :link/completed :handler link-completed}}]
      {})))

{: build}
