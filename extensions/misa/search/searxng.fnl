;; SearXNG backend for a self-hosted, keyless JSON search endpoint. The
;; instance URL is required because a wrong default would silently query a
;; stranger's deployment.

(local search (require :misa.search))

(fn results [payload]
  (let [items (and (= (type payload) :table) (. payload :results))]
    (icollect [_ item (ipairs (if (= (type items) :table) items []))]
      {:snippet item.content :title item.title :url item.url})))

(fn request [settings arguments id]
  "Describe a SearXNG JSON search request."
  (let [base settings.url]
    (assert (and (= (type base) :string) (not= base ""))
            "web_search searxng backend requires config.tools.web_search.searxng.url")
    {:completion :tool/web-search-complete
     :headers [{:name :accept :value :application/json}]
     : id
     :method :GET
     :response_format :json
     :timeouts settings.timeouts
     :type :http/request
     :url (.. base (if (base:find "?" 1 true) "&" "?") :q=
              (search.encode arguments.query) :&format=json)}))

(fn complete [_settings event]
  "Normalize a SearXNG search completion into tool text."
  (if (not event.ok)
      (search.failure event)
      (let [found (results event.data)]
        (if (> (length found) 0)
            (search.success (search.join-results found))
            {:is_error true :text "no web results found"}))))

(fn backend []
  "Create the SearXNG JSON search backend."
  {:build (fn [settings arguments id] (request settings arguments id))
   : complete})

{: backend}
