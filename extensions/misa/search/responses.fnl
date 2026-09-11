;; A search backend built on a provider's Responses API. OpenAI exposes a
;; hosted `web_search` tool there, so a ChatGPT subscription (Codex) or an
;; OpenAI API key can answer a search without a separate search service.

(local search (require :misa.search))

(fn answer [data]
  "Collect assistant text and URL citations from a Responses payload."
  (let [chunks []
        sources []]
    (each [_ item (ipairs (or (and (= (type data) :table) data.output) []))]
      (when (= item.type :message)
        (each [_ block (ipairs (or item.content []))]
          (when (= block.type :output_text)
            (when (= (type block.text) :string)
              (table.insert chunks block.text))
            (each [_ annotation (ipairs (or block.annotations []))]
              (when (and (= annotation.type :url_citation)
                         (= (type annotation.url) :string))
                (table.insert sources
                              {:title annotation.title :url annotation.url})))))))
    {: sources :text (table.concat chunks "\n")}))

;; Backend settings override the model, endpoint, instructions, tool choice, and
;; hosted tool list, so another Responses-compatible provider or a future hosted
;; search tool is configuration rather than a code change.
(fn request [spec settings arguments id]
  "Describe a provider Responses request that uses the hosted web_search tool."
  (let [model (or settings.model spec.model)]
    (assert (= (type model) :string) "provider web search requires a model")
    {:completion :tool/web-search-complete
     :credential spec.credential
     :headers spec.headers
     : id
     :json {:input arguments.query
            :instructions (or settings.instructions spec.instructions
                              "Search the web and answer with the most relevant findings.")
            : model
            :store false
            :stream false
            :tool_choice (or settings.tool_choice spec.tool_choice :required)
            :tools (or settings.tools spec.tools [{:type :web_search}])}
     :method :POST
     :response_format :json
     :timeouts settings.timeouts
     :type :http/request
     :url (or settings.url spec.url)}))

(fn complete [_settings event]
  "Normalize a provider web-search completion into tool text."
  (if (not event.ok)
      (search.failure event)
      (let [payload (answer event.data)]
        (if (= payload.text "")
            {:is_error true :text "web search returned no answer"}
            (search.success (.. payload.text
                                (search.render-sources payload.sources)))))))

(fn backend [spec]
  "Create a search backend that runs a provider's hosted web search."
  {:build (fn [settings arguments id] (request spec settings arguments id))
   : complete})

{: backend}
