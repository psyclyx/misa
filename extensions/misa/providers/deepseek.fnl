;; DeepSeek's OpenAI-compatible chat completions.
;;
;; DeepSeek's model list reports identifiers only, so capability, price, and
;; reasoning facts for known SKUs are declared here and merged onto discovered
;; catalogue rows. Published facts come from
;; https://api-docs.deepseek.com/quick_start/pricing and the Thinking Mode guide:
;; prices below are off-peak rates, and peak hours double them.

(local options (require :misa.providers.openai-options))

(local peak {:multiplier 2
             :weekdays [2 3 4 5 6]
             :windows [{:end_hour 4 :start_hour 1}
                       {:end_hour 10 :start_hour 6}]})

(local flash-pricing {:cache_read 0.003 :input 0.15 :output 0.6 : peak})

(local pro-pricing {:cache_read 0.022 :input 0.66 :output 1.98 : peak})

;; DeepSeek advertises one reasoning-effort ladder for the V4 family, where
;; `none` disables thinking mode.
(local reasoning-efforts [:none :low :high :max])

(local skus {:deepseek-flash {:context_window 1000000
                              :efforts reasoning-efforts
                              :pricing flash-pricing}
             ;; Retired V4 Flash names are served by V4.1 Flash at its price.
             :deepseek-v4-flash {:context_window 1000000
                                 :efforts reasoning-efforts
                                 :pricing flash-pricing}
             :deepseek-v4-flash-vision-exp {:context_window 1000000
                                            :efforts reasoning-efforts
                                            :pricing flash-pricing}
             :deepseek-v4-pro {:context_window 1000000
                               :efforts reasoning-efforts
                               :pricing pro-pricing}})

(fn request-options [sku]
  "Describe DeepSeek request options for a declared SKU.

  `reasoning_effort` both selects the thinking effort and, as `none`, disables
  thinking mode. DeepSeek rejects forced tool choice while thinking, so the
  serializer omits `tool_choice` entirely."
  {:request_options {:reasoning_effort {:choices sku.efforts :default :high}}
   :request_options_serializer :openai.chat.deepseek})

(fn generation [name value]
  "Serialize DeepSeek chat-completions generation options."
  (match name
    :temperature {:temperature value}
    :top_p {:top_p value}
    :stop {:stop value}
    :response_format {:response_format value}
    :max_tokens {:max_tokens value}
    :reasoning_effort {:reasoning_effort value}
    _ nil))

(local serializer (options.compose [generation]))

(fn enrichment []
  "Describe catalogue facts for known DeepSeek model identifiers.

  Unknown identifiers keep their discovered row without facts rather than
  inheriting invented costs or limits."
  (icollect [model sku (pairs skus)]
    {:api (request-options sku)
     :context_window sku.context_window
     :id (.. :deepseek/ model)
     :pricing sku.pricing}))

(fn settings [config]
  "Describe DeepSeek's OpenAI-compatible transport.

  DeepSeek emits no stream bytes while the model reasons, so reasoning warm-ups
  default to a five-minute first-byte and idle budget."
  {:catalogue_authoritative true
   :credential :deepseek
   :id :deepseek
   :max_tokens config.max_tokens
   :max_tokens_field :max_tokens
   :models_url (or config.models_url "https://api.deepseek.com/models")
   :reasoning_content_field :reasoning_content
   :timeouts (misa.patch {:first_byte_ms 300000 :idle_ms 300000}
                         (or config.timeouts {}))
   :url (or config.url "https://api.deepseek.com/chat/completions")})

{: enrichment : request-options : serializer : settings : skus}
