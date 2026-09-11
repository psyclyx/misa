(local search (require :misa.search))
(local responses (require :misa.search.responses))
(local brave (require :misa.search.brave))
(local tavily (require :misa.search.tavily))
(local searxng (require :misa.search.searxng))

;; Model providers that expose the hosted Responses web_search tool are search
;; backends in their own right, so a Codex subscription needs no extra key.
(local codex-spec
       {:credential {:header :authorization
                     :id :openai-codex
                     :metadata_field :account_id
                     :metadata_header :chatgpt-account-id
                     :prefix "Bearer "}
        :headers [{:name :accept :value :application/json}
                  {:name :content-type :value :application/json}
                  {:name :openai-beta :value :responses=experimental}
                  {:name :originator :value :misa}
                  {:name :user-agent :value :misa/0.1}]
        :model :gpt-5.3-codex
        :url "https://chatgpt.com/backend-api/codex/responses"})

(local openai-spec
       {:credential {:header :authorization :id :openai :prefix "Bearer "}
        :headers [{:name :content-type :value :application/json}]
        :model :gpt-5
        :url "https://api.openai.com/v1/responses"})

{:auth-providers {:brave {:description "Brave Search API key"
                          :id :brave
                          :label "Brave Search"
                          :model_provider :brave
                          :profile {:provision_url "https://api-dashboard.search.brave.com/app/keys"}
                          :strategy :api_key}
                  :tavily {:description "Tavily API key"
                           :id :tavily
                           :label :Tavily
                           :model_provider :tavily
                           :profile {:provision_url "https://app.tavily.com/home"}
                           :strategy :api_key}}
 :effects {:tool.web-search/run (fn [effect cofx]
                                  (search.effect (misa.catalog :search-backends)
                                                 cofx.config effect))}
 :events {:web_search/tool/web-search-complete {:event :tool/web-search-complete
                                                :handler (fn [db event cofx]
                                                           (search.completed (misa.catalog :search-backends)
                                                                             cofx.config
                                                                             event))
                                                :priority 12500}}
 :search-backends {:brave (brave.backend)
                   :codex (responses.backend codex-spec)
                   :openai (responses.backend openai-spec)
                   :searxng (searxng.backend)
                   :tavily (tavily.backend)}
 :tools {:web_search {:description "Search the web and return ranked results with titles, URLs, and sources. Configuration selects the backend; Codex, OpenAI, Brave, Tavily, and SearXNG are available."
                      :effect :tool.web-search/run
                      :input_schema {:additionalProperties false
                                     :properties {:max_results {:description "Maximum results to return (defaults to the backend's setting)"
                                                                :maximum 10
                                                                :minimum 1
                                                                :type :integer}
                                                  :query {:description "Search query"
                                                          :type :string}}
                                     :required [:query]
                                     :type :object}
                      :name :web_search}}
 :validators {:search-backends search.validate-backend}}
