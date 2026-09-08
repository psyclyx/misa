;; External links use the ordinary process transport with a configurable opener.
{:setup (fn [context]
          (local config (or context.config.links {}))
          (local command (or config.command ["xdg-open"]))
          (assert (and (= (type command) :table) (> (length command) 0))
                  "links.command must be a nonempty argv array")
          (each [_ argument (ipairs command)]
            (assert (and (= (type argument) :string) (not= argument "")
                         (not (argument:find "\000" 1 true))) "invalid link opener argument"))
          {:fx [{:type :register/event :name :link/open
                 :handler (fn [db event]
                            (assert (and (= (type event.url) :string)
                                         (event.url:match "^https?://[^%s]+$")
                                         (not (event.url:find "\000" 1 true)))
                                    "link URL must use HTTP or HTTPS")
                            (local sequence (+ (or db.link_sequence 0) 1))
                            (local argv (misa.snapshot command))
                            (table.insert argv event.url)
                            {:patch {:link_sequence sequence}
                             :fx [{:type :process/run :id (.. "link:" sequence)
                                   :completion :link/completed :argv argv}
                                  {:type :terminal/read}]})}
                {:type :register/event :name :link/completed
                 :handler (fn [_ event]
                            (when (not event.ok)
                              {:fx [{:type :dispatch
                                     :event {:type :transcript/harness :level :error
                                             :text "Could not open the external link; configure links.command for your browser."}}
                                    {:type :terminal/read}]}))}]})}
