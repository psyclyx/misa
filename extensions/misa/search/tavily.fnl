;; Tavily search API backend. The API key stays native behind the HTTP
;; credential reference.

(local search (require :misa.search))

(fn results [payload]
  (let [items (and (= (type payload) :table) (. payload :results))]
    (icollect [_ item (ipairs (if (= (type items) :table) items []))]
      {:snippet item.content :title item.title :url item.url})))

(fn request [settings arguments id]
  "Describe a Tavily search request."
  {:completion :tool/web-search-complete
   :credential {:header :authorization :id :tavily :prefix "Bearer "}
   :headers [{:name :content-type :value :application/json}]
   : id
   :json {:max_results (or arguments.max_results (or settings.max_results 5))
          :query arguments.query}
   :method :POST
   :response_format :json
   :timeouts settings.timeouts
   :type :http/request
   :url (or settings.url "https://api.tavily.com/search")})

(fn complete [_settings event]
  "Normalize a Tavily search completion into tool text."
  (if (not event.ok)
      (search.failure event)
      (let [payload (and (= (type event.data) :table) event.data)
            found (results payload)
            answer (and payload (= (type payload.answer) :string)
                        (not= payload.answer "") payload.answer)]
        (if (> (length found) 0)
            (search.success (.. (or answer "") (if answer "\n\n" "")
                                (search.join-results found)))
            {:is_error true :text "no web results found"}))))

(fn backend []
  "Create the Tavily search API backend."
  {:build (fn [settings arguments id] (request settings arguments id))
   : complete})

{: backend}
