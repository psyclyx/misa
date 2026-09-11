;; Brave Search API backend. The subscription token stays native: the HTTP
;; effect carries only the credential reference and header name.

(local search (require :misa.search))

(fn results [payload]
  (let [web (and (= (type payload) :table) (. payload :web))
        items (and (= (type web) :table) (. web :results))]
    (icollect [_ item (ipairs (if (= (type items) :table) items []))]
      {:snippet item.description :title item.title :url item.url})))

(fn request [settings arguments id]
  "Describe a Brave Search API request."
  (let [count (or arguments.max_results (or settings.max_results 5))]
    {:completion :tool/web-search-complete
     :credential {:header :x-subscription-token :id :brave :prefix ""}
     :headers [{:name :accept :value :application/json}]
     : id
     :method :GET
     :response_format :json
     :timeouts settings.timeouts
     :type :http/request
     :url (.. (or settings.url "https://api.search.brave.com/res/v1/web/search")
              :?q= (search.encode arguments.query) :&count= (tostring count))}))

(fn complete [_settings event]
  "Normalize a Brave Search completion into tool text."
  (if (not event.ok)
      (search.failure event)
      (let [found (results event.data)]
        (if (> (length found) 0)
            (search.success (search.join-results found))
            {:is_error true :text "no web results found"}))))

(fn backend []
  "Create the Brave Search API backend."
  {:build (fn [settings arguments id] (request settings arguments id))
   : complete})

{: backend}
