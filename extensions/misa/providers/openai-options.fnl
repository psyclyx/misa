;; Reusable request-option fragments for OpenAI-compatible transports. Protocols
;; consume their composed serializer; providers decide which fragments to include.

(fn standard [name value]
  "Serialize OpenAI chat-completions generation options."
  (match name
    :temperature {:temperature value}
    :top_p {:top_p value}
    :top_k {:top_k value}
    :seed {:seed value}
    :presence_penalty {:presence_penalty value}
    :frequency_penalty {:frequency_penalty value}
    :repetition_penalty {:repetition_penalty value}
    :stop {:stop value}
    :tool_choice {:tool_choice value}
    :parallel_tool_calls {:parallel_tool_calls value}
    :response_format {:response_format value}
    :max_tokens {:max_completion_tokens value}
    :reasoning_effort {:reasoning_effort value}
    _ nil))

(fn compose [fragments]
  "Compose provider-owned option fragments into one serializer declaration."
  (fn matching [name value]
    (accumulate [found nil _ fragment (ipairs fragments) &until found]
      (fragment name value)))
  {:accepts (fn [name] (not= (matching name nil) nil))
   :serialize (fn [name value]
                (assert (matching name value)
                        (.. "unsupported OpenAI-compatible option: " name)))})

{:compose compose :standard standard}
