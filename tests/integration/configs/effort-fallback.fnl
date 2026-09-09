(let [config {"models" {"default" "fake/wide"}
              "providers" {"fake" {"expect_request_options" {"reasoning_effort" "medium"}
                                   "expect_request_options_exact" true
                                   "responses" ["fallback"]}}}
      app ((require :tests.application) {:config config})]
  (app.include (. (require :tests.stock) :misa.agent.stream))
  (app.include (. (require :tests.stock) :misa.keybindings))
  (app.include (. (require :tests.stock) :misa.ui.values))
  (app.include (. (require :tests.stock) :misa.providers.fake))
  (app.include (. (require :tests.stock) :misa.ui.themes))
  (app.include (. (require :tests.stock) :misa.ui.themes.default))
  (app.include (. (require :tests.stock) :misa.ui.components))
  (app.include (. (require :tests.stock) :misa.ui.layout))
  (app.include (. (require :tests.stock) :misa.markdown))
  (app.include (. (require :tests.stock) :misa.markdown.render))
  (app.include (. (require :tests.stock) :misa.ui.components.content))
  (app.include (. (require :tests.stock) :misa.ui.components.truncation))
  (app.include (. (require :tests.stock) :misa.transcript.tools))
  (app.include (. (require :tests.stock) :misa.transcript.tools.render))
  (app.include (. (require :tests.stock) :misa.ui.components.group))
  (app.include (. (require :tests.stock) :misa.transcript.render))
  (app.include (. (require :tests.stock) :misa.editor.render))
  (app.include (. (require :tests.stock) :misa.choices.picker.render))
  (app.include (. (require :tests.stock) :misa.ui.status.render))
  (app.include (. (require :tests.stock) :misa.ui.chrome))
  (app.include (. (require :tests.stock) :misa.transcript))
  (app.include (. (require :tests.stock) :misa.models))
  (app.include (. (require :tests.stock) :misa.models.options))
  (app.include (. (require :tests.stock) :misa.agent))
  (app.include (. (require :tests.stock) :misa.commands))
  (app.include (. (require :tests.stock) :misa.choices))
  (app.include (. (require :tests.stock) :misa.editor))
  (app.include (. (require :tests.stock) :misa.ui))
  (app.include (. (require :tests.stock) :misa.transcript.groups))
  (each [id model (pairs app.definitions.models)]
    (when (= model.provider :fake) (tset app.definitions.models id nil)))
  (tset app.definitions.models "fake/wide"
        {"id" "fake/wide"
         "model" "wide"
         "api" {"request_options" {"reasoning_effort" {"choices" ["low" "high"]
                                                       "default" "high"}}
                :request_options_serializer :fake.options}
         :provider :fake})
  (tset app.definitions.models "fake/narrow"
        {"id" "fake/narrow"
         "model" "narrow"
         "api" {"request_options" {"reasoning_effort" {"choices" ["low"
                                                                  "medium"]
                                                       "default" "medium"}}
                :request_options_serializer :fake.options}
         :provider :fake})
  {:config config :definitions app.definitions})
