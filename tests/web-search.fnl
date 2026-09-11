(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local search (require :misa.search))
(local fragment (require :misa.standard.tools.web-search))
(local backends fragment.search-backends)

(fn fails? [f expected]
  (let [(ok message) (pcall f)]
    (assert (and (not ok) (: (tostring message) :find expected 1 true)))))

;; Settings default to Codex and tolerate absent or malformed sections.
(assert (= (search.selected {}) :codex))
(assert (= (search.selected {:tools {:web_search {:backend :searxng}}}) :searxng))
(assert (= (next (search.configuration {})) nil))
(assert (= (next (search.configuration {:tools {}})) nil))
(assert (= (next (search.configuration {:tools {:web_search "bad"}})) nil))

;; Argument validation is shared by every backend.
(let [described (search.describe {:arguments {:query "a b" :max_results 3}})]
  (assert (= described.query "a b"))
  (assert (= described.max_results 3)))
(fails? (fn [] (search.describe {:arguments {:query ""}})) "nonempty string")
(fails? (fn [] (search.describe {:arguments {:query 3}})) "nonempty string")
(fails? (fn [] (search.describe {:arguments {}})) "nonempty string")
(fails? (fn [] (search.describe {:arguments "q"})) "object")
(fails? (fn []
          (search.describe {:arguments {:query "q" :max_results 11}}))
        "max_results")
(fails? (fn []
          (search.describe {:arguments {:query "q" :max_results 0}}))
        "max_results")
(fails? (fn []
          (search.describe {:arguments {:query "q" :max_results 1.5}}))
        "max_results")

(assert (= (search.encode "a b/c+d") "a%20b%2Fc%2Bd"))
(assert (= (search.encode "safe-_.~") "safe-_.~"))
(assert (= (search.join-results [{:title "T" :url "u" :snippet "s"}]) "T\nu\ns"))
(assert (= (search.join-results [{:url "u"}]) "u\nu"))
(assert (= (search.render-sources [{:url "u"}]) "\n\nSources:\n- u"))
(assert (= (search.render-sources []) ""))

;; The default Codex backend asks a provider's Responses API for the hosted
;; web_search tool, reusing the ChatGPT subscription credential.
(let [effect (search.effect backends {}
                            {:arguments {:query "zig build"}
                             :tool_call_id :call-1})]
  (assert (= effect.type :http/request))
  (assert (= effect.method :POST))
  (assert (= effect.completion :tool/web-search-complete))
  (assert (= effect.id :call-1))
  (assert (= effect.response_format :json))
  (assert (= effect.credential.id :openai-codex))
  (assert (= effect.credential.metadata_header :chatgpt-account-id))
  (assert (= effect.url "https://chatgpt.com/backend-api/codex/responses"))
  (assert (= (. effect.json.tools 1 :type) :web_search))
  (assert (= effect.json.tool_choice :required))
  (assert (= effect.json.input "zig build"))
  (assert (= effect.json.model "gpt-5.3-codex"))
  (assert (= effect.json.stream false))
  (assert (not effect.json.store)))

;; Per-backend settings override the model, endpoint, and hosted-tool request,
;; and each backend selects a different transport shape without changing the
;; tool contract.
(let [effect (search.effect backends {:tools {:web_search {:backend :openai
                                                         :openai {:model "gpt-5-mini"
                                                                  :url "https://gateway.test/v1/responses"
                                                                  :instructions "Search briefly."
                                                                  :tool_choice :auto
                                                                  :tools [{:type :web_search_preview}]}}}}
                            {:arguments {:query "q"} :tool_call_id :call-2})]
  (assert (= effect.credential.id :openai))
  (assert (= effect.url "https://gateway.test/v1/responses"))
  (assert (= effect.json.model "gpt-5-mini"))
  (assert (= effect.json.instructions "Search briefly."))
  (assert (= effect.json.tool_choice :auto))
  (assert (= (. effect.json.tools 1 :type) :web_search_preview)))

(let [effect (search.effect backends {:tools {:web_search {:backend :brave
                                                          :max_results 3}}}
                            {:arguments {:query "c++ std::map"}
                             :tool_call_id :call-3})]
  (assert (= effect.method :GET))
  (assert (= effect.credential.id :brave))
  (assert (= effect.credential.header :x-subscription-token))
  (assert (= effect.credential.prefix ""))
  (assert (effect.url:find "q=c%2B%2B%20std%3A%3Amap" 1 true))
  (assert (effect.url:find "count=3" 1 true)))

(let [effect (search.effect backends {:tools {:web_search {:backend :tavily}}}
                            {:arguments {:query "x" :max_results 2}
                             :tool_call_id :call-4})]
  (assert (= effect.method :POST))
  (assert (= effect.credential.id :tavily))
  (assert (= effect.json.query "x"))
  (assert (= effect.json.max_results 2)))

;; A keyless backend still requires an explicit instance.
(fails? (fn []
          (search.effect backends {:tools {:web_search {:backend :searxng}}}
                         {:arguments {:query "x"} :tool_call_id :call-5}))
        "searxng")
(let [effect (search.effect backends {:tools {:web_search {:backend :searxng
                                                          :searxng {:url "https://searx.example/"}}}}
                            {:arguments {:query "a b"} :tool_call_id :call-6})]
  (assert (= effect.url "https://searx.example/?q=a%20b&format=json")))

(fails? (fn []
          (search.effect backends {:tools {:web_search {:backend :missing}}}
                         {:arguments {:query "x"} :tool_call_id :call-7}))
        "not installed")

;; The configured backend reads arguments without mutating them.
(let [config {:tools {:web_search {:backend :brave}}}
      arguments {:query "q" :max_results 4}]
  (search.effect backends config {:arguments arguments :tool_call_id :call-8})
  (assert (= config.tools.web_search.backend :brave))
  (assert (= config.tools.web_search.brave nil))
  (assert (= arguments.query "q"))
  (assert (= arguments.max_results 4)))

;; A Responses completion becomes an answer with cited sources.
(let [event {:data {:output [{:content [{:annotations [{:title "A"
                                                         :type :url_citation
                                                         :url "https://a.example"}]
                                          :text "Answer text"
                                          :type :output_text}]
                               :type :message}]}
             :id :call-1
             :ok true}
      result (search.completed backends {} event)
      event-out (. result.fx 1 :event)]
  (assert (= event-out.type :tool/result))
  (assert (= event-out.tool_call_id :call-1))
  (assert (= event-out.is_error false))
  (assert (event-out.text:find "Answer text" 1 true))
  (assert (event-out.text:find "https://a.example" 1 true)))

;; Dedicated search backends render their own result shapes.
(let [result ((. backends :brave :complete) {}
              {:data {:web {:results [{:description "S1" :title "T1" :url "u1"}
                                      {:description "S2" :title "T2" :url "u2"}]}}
               :id :b :ok true})]
  (assert (= result.is_error false))
  (assert (result.text:find "T1\nu1\nS1" 1 true))
  (assert (result.text:find "\n\nT2\nu2\nS2" 1 true)))

(let [result ((. backends :tavily :complete) {}
              {:data {:answer "Direct answer"
                      :results [{:content "C" :title "T" :url "u"}]}
               :id :t :ok true})]
  (assert (= result.is_error false))
  (assert (result.text:find "Direct answer\n\nT\nu\nC" 1 true)))

(let [result ((. backends :searxng :complete) {}
              {:data {:results [{:content "C" :title "T" :url "u"}]}
               :id :s :ok true})]
  (assert (= result.is_error false))
  (assert (result.text:find "T\nu\nC" 1 true)))

(let [result ((. backends :brave :complete) {} {:data {} :id :e :ok true})]
  (assert result.is_error)
  (assert (= result.text "no web results found")))

(let [result ((. backends :codex :complete) {} {:id :f :ok false :status 500})]
  (assert result.is_error)
  (assert (= result.text "web search failed with status 500")))

(let [result ((. backends :codex :complete) {}
              {:data {:output []} :id :g :ok true})]
  (assert result.is_error)
  (assert (= result.text "web search returned no answer")))

;; A backend entry is validated when the application installs.
(fails? (fn [] (search.validate-backend :bad {})) "search backend")
(fails? (fn [] (search.validate-backend :bad {:build (fn [] nil)})) "search backend")
(search.validate-backend :brave (. backends :brave))

;; The stock fragment declares the tool, its effect, and its event handler.
(assert (= (. fragment.tools :web_search :effect) :tool.web-search/run))
(assert (= (. fragment.tools :web_search :input_schema :type) :object))
(assert (= (length (. fragment.tools :web_search :input_schema :required)) 1))
(assert (= (type (. fragment.effects :tool.web-search/run)) :function))
(assert (= (type (. fragment.events :web_search/tool/web-search-complete :handler))
           :function))
(assert (= (. fragment.auth-providers :brave :strategy) :api_key))
(assert (= (. fragment.auth-providers :tavily :strategy) :api_key))
(assert (not= (. fragment.search-backends :brave) nil))
(assert (not= (. fragment.search-backends :codex) nil))
(assert (not= (. fragment.search-backends :openai) nil))
(assert (not= (. fragment.search-backends :searxng) nil))
(assert (not= (. fragment.search-backends :tavily) nil))

;; Installed wiring resolves the backend catalog and reaches the same request.
(let [app ((require :tests.application)
           {:argv [] :config {:tools {:web_search {:backend :codex}}}})]
  (app.include fragment)
  (app.install)
  (assert (= (type (. (misa.catalog :search-backends) :codex)) :table))
  (assert (= (. (misa.tools.lookup :web_search) :name) :web_search))
  (let [effect ((. fragment.effects :tool.web-search/run)
                {:arguments {:query "installed"}
                 :name :web_search
                 :tool_call_id :call-9}
                {:config {:tools {:web_search {:backend :codex}}}})]
    (assert (= effect.type :http/request))
    (assert (= effect.id :call-9))
    (assert (= effect.json.input "installed")))
  (let [result ((. fragment.events :web_search/tool/web-search-complete :handler)
                {}
                {:data {:output [{:content [{:text "installed answer"
                                            :type :output_text}]
                                 :type :message}]}
                 :id :call-9
                 :ok true}
                {:config {}})]
    (assert (= (. result.fx 1 :event :text) "installed answer"))
    (assert (= (. result.fx 1 :event :type) :tool/result))))

(output "web search policy contracts passed\n")
