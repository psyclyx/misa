;; Web search spans two kinds of provider: model providers whose Responses API
;; exposes a hosted web_search tool (notably Codex), and dedicated search
;; services. Both are ordinary entries in the open `search-backends` catalog, so
;; this policy names no vendor and chooses a backend only from configuration.

(fn configuration [config]
  "Return the web-search settings, defaulting to an empty map."
  (let [tools (and (= (type config) :table) config.tools)
        section (and (= (type tools) :table) tools.web_search)]
    (if (= (type section) :table) section {})))

(fn selected [config]
  "Return the configured search backend ID."
  (let [backend (. (configuration config) :backend)]
    (if (= backend nil) :codex backend)))

(fn backend [backends id]
  "Look up an installed search backend by ID."
  (let [found (and (= (type backends) :table) (. backends id))]
    (assert (= (type found) :table)
            (.. "web search backend is not installed: " (tostring id)))
    found))

(fn describe [effect]
  "Validate a web-search tool call into normalized arguments."
  (let [arguments (assert (and (= (type effect.arguments) :table)
                               effect.arguments)
                          "web_search arguments must be an object")
        query arguments.query]
    (assert (and (= (type query) :string) (not= query "")
                 (not (query:find "\000" 1 true)))
            "query must be a nonempty string")
    (let [limit arguments.max_results]
      (when (not= limit nil)
        (assert (and (= (type limit) :number) (= (% limit 1) 0) (>= limit 1)
                     (<= limit 10))
                "max_results must be an integer from 1 to 10")))
    {:max_results arguments.max_results : query}))

(fn options [settings id]
  (let [section (and (= (type settings) :table) (. settings id))]
    (if (= (type section) :table) (misa.patch settings section) settings)))

(fn effect [backends config tool-effect]
  "Describe the native request for a web-search tool call."
  (let [settings (configuration config)
        id (selected config)
        entry (backend backends id)]
    (entry.build (options settings id) (describe tool-effect)
                 tool-effect.tool_call_id)))

(fn completed [backends config event]
  "Translate a search completion into a tool result."
  (let [settings (configuration config)
        id (selected config)
        result ((. (backend backends id) :complete) (options settings id) event)]
    {:fx [{:type :dispatch
           :event {:is_error (= result.is_error true)
                   :text (tostring result.text)
                   :tool_call_id event.id
                   :type :tool/result}}]}))

(fn success [text]
  "Describe a successful search result."
  {:is_error false : text})

(fn failure [event]
  "Describe a failed search request using its transport outcome."
  {:is_error true
   :text (or event.message
             (.. "web search failed with status " (tostring event.status)))})

(fn encode [value]
  "Percent-encode one query-string component."
  (value:gsub "[^%w%-%_%.%~]"
              (fn [character]
                (.. "%" (string.format "%02X" (character:byte))))))

(fn join-results [results]
  "Render ordered result records as readable tool text."
  (let [blocks (icollect [_ result (ipairs (or results []))]
                 (let [lines [(or result.title result.url :result)]]
                   (when result.url (table.insert lines result.url))
                   (when result.snippet (table.insert lines result.snippet))
                   (table.concat lines "\n")))]
    (table.concat blocks "\n\n")))

(fn render-sources [sources]
  "Render cited sources as a trailing list."
  (let [lines (icollect [_ source (ipairs (or sources []))]
                (when source.url (.. "- " source.url)))]
    (if (> (length lines) 0)
        (.. "\n\nSources:\n" (table.concat lines "\n"))
        "")))

(fn validate-backend [_id value]
  "Require a search backend with request and completion operations."
  (assert (and (= (type value) :table) (= (type value.build) :function)
               (= (type value.complete) :function))
          "search backend requires build and complete functions"))

{: configuration
 : selected
 : backend
 : describe
 : effect
 : completed
 : success
 : failure
 : encode
 : join-results
 : render-sources
 : validate-backend}
