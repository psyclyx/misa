;; Fixture subsets are ordinary declaration maps; stock event order is tested
;; separately from tests that deliberately install modules in a chosen order.
(local catalogs
       {:misa.actions (require :misa.standard.actions)
        :misa.agent (require :misa.standard.agent)
        :misa.agent.stream (require :misa.standard.agent.stream)
        :misa.choices (require :misa.standard.choices)
        :misa.choices.layout (require :misa.standard.choices.layout)
        :misa.choices.matching (require :misa.standard.choices.matching)
        :misa.choices.picker (require :misa.standard.choices.picker)
        :misa.choices.picker.render (require :misa.standard.choices.picker.render)
        :misa.choices.picker.view (require :misa.standard.choices.picker.view)
        :misa.choices.preferences (require :misa.standard.choices.preferences)
        :misa.choices.preview (require :misa.standard.choices.preview)
        :misa.clipboard (require :misa.standard.clipboard)
        :misa.commands (require :misa.standard.commands)
        :misa.commands.palette (require :misa.standard.commands.palette)
        :misa.costs (require :misa.standard.costs)
        :misa.dialogs (require :misa.standard.dialogs)
        :misa.dialogs.render (require :misa.standard.dialogs.render)
        :misa.dialogs.view (require :misa.standard.dialogs.view)
        :misa.editor (require :misa.standard.editor)
        :misa.editor.attachments (require :misa.standard.editor.attachments)
        :misa.editor.editing (require :misa.standard.editor.editing)
        :misa.editor.history (require :misa.standard.editor.history)
        :misa.editor.images (require :misa.standard.editor.images)
        :misa.editor.images.render (require :misa.standard.editor.images.render)
        :misa.editor.queue (require :misa.standard.editor.queue)
        :misa.editor.queue.view (require :misa.standard.editor.queue.view)
        :misa.editor.render (require :misa.standard.editor.render)
        :misa.json (require :misa.standard.json)
        :misa.keybindings (require :misa.standard.keybindings)
        :misa.links (require :misa.standard.links)
        :misa.markdown (let [stock (require :misa.standard.presentation.markdown)]
                         {:services {:markdown (. stock :services :markdown)}})
        :misa.markdown.render (let [stock (require :misa.standard.presentation.markdown)]
                                {:requirements {:component.markdown (. stock
                                                                       :requirements
                                                                       :component.markdown)}
                                 :services {:markdown.view (. stock :services
                                                              :markdown.view)}})
        :misa.models (require :misa.standard.models)
        :misa.models.effort (require :misa.standard.models.effort)
        :misa.models.options (require :misa.standard.models.options)
        :misa.models.preview (require :misa.standard.models.preview)
        :misa.protocols.anthropic (require :misa.standard.protocols.anthropic)
        :misa.protocols.openai (require :misa.standard.protocols.openai)
        :misa.providers.anthropic (require :misa.standard.providers.anthropic)
        :misa.providers.auth (require :misa.standard.providers.auth)
        :misa.providers.cerebras (require :misa.standard.providers.cerebras)
        :misa.providers.claude (require :misa.standard.providers.claude)
        :misa.providers.deepseek (require :misa.standard.providers.deepseek)
        :misa.providers.deepinfra (require :misa.standard.providers.deepinfra)
        :misa.providers.fireworks (require :misa.standard.providers.fireworks)
        :misa.providers.generic (require :misa.standard.providers.generic)
        :misa.providers.groq (require :misa.standard.providers.groq)
        :misa.providers.huggingface (require :misa.standard.providers.huggingface)
        :misa.providers.command (require :misa.standard.providers.command)
        :misa.providers.fake (require :misa.standard.providers.fake)
        :misa.providers.kimi (require :misa.standard.providers.kimi)
        :misa.providers.mistral (require :misa.standard.providers.mistral)
        :misa.providers.moonshot (require :misa.standard.providers.moonshot)
        :misa.providers.novita (require :misa.standard.providers.novita)
        :misa.providers.nvidia (require :misa.standard.providers.nvidia)
        :misa.providers.openai (require :misa.standard.providers.openai)
        :misa.providers.openai-codex (require :misa.standard.providers.openai-codex)
        :misa.providers.openrouter (require :misa.standard.providers.openrouter)
        :misa.providers.siliconflow (require :misa.standard.providers.siliconflow)
        :misa.providers.together (require :misa.standard.providers.together)
        :misa.providers.venice (require :misa.standard.providers.venice)
        :misa.providers.xai (require :misa.standard.providers.xai)
        :misa.selection (require :misa.standard.selection)
        :misa.selection.document (require :misa.standard.selection.document)
        :misa.selection.render (require :misa.standard.selection.render)
        :misa.tools.files (require :misa.standard.tools.files)
        :misa.tools.shell (require :misa.standard.tools.shell)
        :misa.transcript (let [stock (require :misa.standard.presentation.transcript)]
                           {:actions {:transcript.detail (. stock :actions
                                                            :transcript.detail)
                                      :transcript.down (. stock :actions
                                                          :transcript.down)
                                      :transcript.up (. stock :actions
                                                        :transcript.up)}
                            :events {:messages/transcript/reset (. stock
                                                                   :events
                                                                   :messages/transcript/reset)
                                     :messages/transcript/response-start (. stock
                                                                            :events
                                                                            :messages/transcript/response-start)
                                     :messages/transcript/block-start (. stock
                                                                         :events
                                                                         :messages/transcript/block-start)
                                     :messages/transcript/block-delta (. stock
                                                                         :events
                                                                         :messages/transcript/block-delta)
                                     :messages/transcript/block-end (. stock
                                                                       :events
                                                                       :messages/transcript/block-end)
                                     :messages/transcript/response-end (. stock
                                                                          :events
                                                                          :messages/transcript/response-end)
                                     :messages/transcript/response-interrupted (. stock
                                                                                  :events
                                                                                  :messages/transcript/response-interrupted)
                                     :messages/transcript/user (. stock :events
                                                                  :messages/transcript/user)
                                     :messages/transcript/harness (. stock
                                                                     :events
                                                                     :messages/transcript/harness)
                                     :messages/transcript/tool-start (. stock
                                                                        :events
                                                                        :messages/transcript/tool-start)
                                     :messages/transcript/tool-result (. stock
                                                                         :events
                                                                         :messages/transcript/tool-result)
                                     :messages/transcript/tool-summary (. stock
                                                                          :events
                                                                          :messages/transcript/tool-summary)
                                     :messages/transcript/tool-call (. stock
                                                                       :events
                                                                       :messages/transcript/tool-call)
                                     :messages/transcript/assistant (. stock
                                                                       :events
                                                                       :messages/transcript/assistant)
                                     :messages/transcript/interrupted (. stock
                                                                         :events
                                                                         :messages/transcript/interrupted)
                                     :messages/runtime/dispatch-limit (. stock
                                                                         :events
                                                                         :messages/runtime/dispatch-limit)
                                     :messages/app/start (. stock :events
                                                            :messages/app/start)
                                     :messages/messages/toggle-verbose (. stock
                                                                          :events
                                                                          :messages/messages/toggle-verbose)
                                     :messages/messages/scroll (. stock :events
                                                                  :messages/messages/scroll)}
                            :indicators {:transcript-detail (. stock
                                                               :indicators
                                                               :transcript-detail)}
                            :keybindings {:global/transcript_up (. stock
                                                                   :keybindings
                                                                   :global/transcript_up)
                                          :global/transcript_down (. stock
                                                                     :keybindings
                                                                     :global/transcript_down)
                                          :global/toggle_verbose (. stock
                                                                    :keybindings
                                                                    :global/toggle_verbose)}
                            :projections {:transcript.project (. stock
                                                                 :projections
                                                                 :transcript.project)}
                            :routes {:messages/global-keys (. stock :routes
                                                              :messages/global-keys)}
                            :selection-sources {:transcript (. stock
                                                               :selection-sources
                                                               :transcript)}
                            :services {:transcript.window (. stock :services
                                                             :transcript.window)
                                       :transcript.viewport (. stock :services
                                                               :transcript.viewport)
                                       :transcript.state (. stock :services
                                                            :transcript.state)
                                       :transcript.blocks (. stock :services
                                                             :transcript.blocks)}
                            :subscriptions {:messages/detail-indicator (. stock
                                                                          :subscriptions
                                                                          :messages/detail-indicator)}
                            :transcript-deltas {:tool_call (. stock
                                                              :transcript-deltas
                                                              :tool_call)
                                                :assistant (. stock
                                                              :transcript-deltas
                                                              :assistant)
                                                :thinking (. stock
                                                             :transcript-deltas
                                                             :thinking)}
                            :transcript-presentations {:tool_result (. stock
                                                                       :transcript-presentations
                                                                       :tool_result)
                                                       :harness (. stock
                                                                   :transcript-presentations
                                                                   :harness)
                                                       :tool_call (. stock
                                                                     :transcript-presentations
                                                                     :tool_call)
                                                       :assistant (. stock
                                                                     :transcript-presentations
                                                                     :assistant)
                                                       :user (. stock
                                                                :transcript-presentations
                                                                :user)
                                                       :thinking (. stock
                                                                    :transcript-presentations
                                                                    :thinking)}
                            :validators {:transcript-deltas (. stock
                                                               :validators
                                                               :transcript-deltas)
                                         :transcript-presentations (. stock
                                                                      :validators
                                                                      :transcript-presentations)}})
        :misa.transcript.groups (let [stock (require :misa.standard.presentation.elements)]
                                  {:components {:default.transcript.group_header (. stock
                                                                                    :components
                                                                                    :default.transcript.group_header)
                                                :default.transcript.group_footer (. stock
                                                                                    :components
                                                                                    :default.transcript.group_footer)}})
        :misa.transcript.render (let [stock (require :misa.standard.presentation.elements)]
                                  {:components {:default.transcript.thinking_collapsed (. stock
                                                                                          :components
                                                                                          :default.transcript.thinking_collapsed)
                                                :default.transcript.user (. stock
                                                                            :components
                                                                            :default.transcript.user)
                                                :default.transcript.harness (. stock
                                                                               :components
                                                                               :default.transcript.harness)
                                                :default.transcript.assistant (. stock
                                                                                 :components
                                                                                 :default.transcript.assistant)
                                                :default.transcript.thinking (. stock
                                                                                :components
                                                                                :default.transcript.thinking)}
                                   :requirements {:component.message (. stock
                                                                        :requirements
                                                                        :component.message)}})
        :misa.transcript.syntax (let [stock (require :misa.standard.presentation.syntax)]
                                  {:events {:syntax/transcript/updated (. stock
                                                                          :events
                                                                          :syntax/transcript/updated)
                                            :syntax/app/start (. stock :events
                                                                 :syntax/app/start)
                                            :syntax/syntax/completed (. stock
                                                                        :events
                                                                        :syntax/syntax/completed)
                                            :syntax/transcript/reset (. stock
                                                                        :events
                                                                        :syntax/transcript/reset)}
                                   :services {:syntax.for-model (. stock
                                                                   :services
                                                                   :syntax.for-model)
                                              :syntax.all (. stock :services
                                                             :syntax.all)}
                                   :subscriptions {:syntax/projections (. stock
                                                                          :subscriptions
                                                                          :syntax/projections)
                                                   :syntax/projection (. stock
                                                                         :subscriptions
                                                                         :syntax/projection)}})
        :misa.transcript.tools (let [stock (require :misa.standard.presentation.tools)]
                                 {:services {:tools.presentation (. stock
                                                                    :services
                                                                    :tools.presentation)}
                                  :tool-presentations {:shell (. stock
                                                                 :tool-presentations
                                                                 :shell)
                                                       :write_file (. stock
                                                                      :tool-presentations
                                                                      :write_file)
                                                       :edit_file (. stock
                                                                     :tool-presentations
                                                                     :edit_file)
                                                       :read_file (. stock
                                                                     :tool-presentations
                                                                     :read_file)
                                                       :list_directory (. stock
                                                                          :tool-presentations
                                                                          :list_directory)}
                                  :validators {:tool-presentations (. stock
                                                                      :validators
                                                                      :tool-presentations)}})
        :misa.transcript.tools.render (let [stock (require :misa.standard.presentation.elements)]
                                        {:components {:default.transcript.tool_call (. stock
                                                                                       :components
                                                                                       :default.transcript.tool_call)
                                                      :default.transcript.tool_result (. stock
                                                                                         :components
                                                                                         :default.transcript.tool_result)}
                                         :requirements {:component.tool (. stock
                                                                           :requirements
                                                                           :component.tool)}})
        :misa.transcript.tools.summary (let [stock (require :misa.standard.presentation.tools)]
                                         {:events {:tool_summary/tool-summary/reconcile (. stock
                                                                                           :events
                                                                                           :tool_summary/tool-summary/reconcile)
                                                   :tool_summary/agent/result (. stock
                                                                                 :events
                                                                                 :tool_summary/agent/result)
                                                   :tool_summary/agent/stream-end (. stock
                                                                                     :events
                                                                                     :tool_summary/agent/stream-end)
                                                   :tool_summary/agent/stream-error (. stock
                                                                                       :events
                                                                                       :tool_summary/agent/stream-error)
                                                   :tool_summary/agent/reset (. stock
                                                                                :events
                                                                                :tool_summary/agent/reset)
                                                   :tool_summary/transcript/reset (. stock
                                                                                     :events
                                                                                     :tool_summary/transcript/reset)
                                                   :tool_summary/transcript/tool-result (. stock
                                                                                           :events
                                                                                           :tool_summary/transcript/tool-result)
                                                   :tool_summary/tool-summary/next (. stock
                                                                                      :events
                                                                                      :tool_summary/tool-summary/next)
                                                   :tool_summary/agent/stream-delta (. stock
                                                                                       :events
                                                                                       :tool_summary/agent/stream-delta)
                                                   :tool_summary/agent/stream-usage (. stock
                                                                                       :events
                                                                                       :tool_summary/agent/stream-usage)
                                                   :tool_summary/model/role (. stock
                                                                               :events
                                                                               :tool_summary/model/role)
                                                   :tool_summary/model/roles-loaded (. stock
                                                                                       :events
                                                                                       :tool_summary/model/roles-loaded)
                                                   :tool_summary/models/update (. stock
                                                                                  :events
                                                                                  :tool_summary/models/update)
                                                   :tool_summary/models/provider-availability (. stock
                                                                                                 :events
                                                                                                 :tool_summary/models/provider-availability)
                                                   :tool_summary/models/replace-provider (. stock
                                                                                            :events
                                                                                            :tool_summary/models/replace-provider)}})
        :misa.ui (let [stock (require :misa.standard.presentation.ui)]
                   {:services {:ui.input-budgets (. stock :services
                                                    :ui.input-budgets)
                               :ui.overlay-room (. stock :services
                                                   :ui.overlay-room)
                               :ui.completion-room (. stock :services
                                                      :ui.completion-room)
                               :ui.picker-room (. stock :services
                                                  :ui.picker-room)
                               :ui.regions (. stock :services :ui.regions)
                               :ui.bound-frame (. stock :services
                                                  :ui.bound-frame)}
                    :views {:main (. stock :views :main)}})
        :misa.ui.animations (let [stock (require :misa.standard.presentation.animations)]
                              {:events {:animations/app/start (. stock :events
                                                                 :animations/app/start)
                                        :animations/animations/loaded (. stock
                                                                         :events
                                                                         :animations/animations/loaded)
                                        :animations/animations/swap (. stock
                                                                       :events
                                                                       :animations/animations/swap)
                                        :animations/animations/start (. stock
                                                                        :events
                                                                        :animations/animations/start)
                                        :animations/animations/stop (. stock
                                                                       :events
                                                                       :animations/animations/stop)
                                        :animations/animations/tick (. stock
                                                                       :events
                                                                       :animations/animations/tick)}
                               :services {:animations.lookup (. stock :services
                                                                :animations.lookup)
                                          :animations.frame (. stock :services
                                                               :animations.frame)
                                          :animations.span (. stock :services
                                                              :animations.span)
                                          :animations.state (. stock :services
                                                               :animations.state)
                                          :animations.swap (. stock :services
                                                              :animations.swap)}
                               :subscriptions {:animations/presentation (. stock
                                                                           :subscriptions
                                                                           :animations/presentation)}
                               :validators {:animations (. stock :validators
                                                           :animations)}})
        :misa.ui.animations.default (let [stock (require :misa.standard.presentation.animations)]
                                      {:animations {:default (. stock
                                                                :animations
                                                                :default)
                                                    :spinner (. stock
                                                                :animations
                                                                :spinner)
                                                    :static (. stock
                                                               :animations
                                                               :static)}})
        :misa.ui.chrome (let [stock (require :misa.standard.presentation.elements)]
                          {:components {:default.root.header (. stock
                                                                :components
                                                                :default.root.header)}})
        :misa.ui.components (let [stock (require :misa.standard.presentation.components)]
                              {:events {:components/app/start (. stock :events
                                                                 :components/app/start)
                                        :components/components/loaded (. stock
                                                                         :events
                                                                         :components/components/loaded)
                                        :components/components/swap (. stock
                                                                       :events
                                                                       :components/components/swap)}
                               :services {:components.resolve (. stock
                                                                 :services
                                                                 :components.resolve)
                                          :components.render (. stock :services
                                                                :components.render)
                                          :components.project (. stock
                                                                 :services
                                                                 :components.project)
                                          :components.lookup (. stock :services
                                                                :components.lookup)
                                          :components.swap (. stock :services
                                                              :components.swap)}
                               :subscriptions {:components/projection (. stock
                                                                         :subscriptions
                                                                         :components/projection)}
                               :validators {:components (. stock :validators
                                                           :components)}})
        :misa.ui.components.buttons (let [stock (require :misa.standard.presentation.elements)]
                                      {:requirements {:component.buttons (. stock
                                                                            :requirements
                                                                            :component.buttons)}
                                       :services {:components.buttons (. stock
                                                                         :services
                                                                         :components.buttons)}})
        :misa.ui.components.content (let [stock (require :misa.standard.presentation.elements)]
                                      {:components {:default.content.diff (. stock
                                                                             :components
                                                                             :default.content.diff)
                                                    :default.content.fields (. stock
                                                                               :components
                                                                               :default.content.fields)
                                                    :default.content.code (. stock
                                                                             :components
                                                                             :default.content.code)
                                                    :default.content.lines (. stock
                                                                              :components
                                                                              :default.content.lines)
                                                    :default.content.text (. stock
                                                                             :components
                                                                             :default.content.text)}
                                       :requirements {:component.content (. stock
                                                                            :requirements
                                                                            :component.content)}})
        :misa.ui.components.data (let [stock (require :misa.standard.presentation.elements)]
                                   {:components {:default.data (. stock
                                                                  :components
                                                                  :default.data)}
                                    :requirements {:component.data (. stock
                                                                      :requirements
                                                                      :component.data)}})
        :misa.ui.components.group (let [stock (require :misa.standard.presentation.elements)]
                                    {:components {:default.group.boundary (. stock
                                                                             :components
                                                                             :default.group.boundary)}
                                     :requirements {:component.group (. stock
                                                                        :requirements
                                                                        :component.group)}})
        :misa.ui.components.truncation (let [stock (require :misa.standard.presentation.elements)]
                                         {:components {:default.content.truncated (. stock
                                                                                     :components
                                                                                     :default.content.truncated)}})
        :misa.ui.layout (let [stock (require :misa.standard.presentation.ui)]
                          {:services {:layout (. stock :services :layout)}})
        :misa.ui.status (let [stock (require :misa.standard.presentation.status)]
                          {:events {:status/app/start (. stock :events
                                                         :status/app/start)
                                    :status/agent/status (. stock :events
                                                            :status/agent/status)}
                           :indicators {:session (. stock :indicators :session)
                                        :plan (. stock :indicators :plan)
                                        :activity (. stock :indicators
                                                     :activity)
                                        :context (. stock :indicators :context)}
                           :services {:status.model (. stock :services
                                                       :status.model)}
                           :subscriptions {:status/plan (. stock :subscriptions
                                                           :status/plan)
                                           :status/context (. stock
                                                              :subscriptions
                                                              :status/context)
                                           :status/activity (. stock
                                                               :subscriptions
                                                               :status/activity)
                                           :status/session (. stock
                                                              :subscriptions
                                                              :status/session)}})
        :misa.ui.status.indicators (let [stock (require :misa.standard.presentation.status)]
                                     {:services {:status.indicators (. stock
                                                                       :services
                                                                       :status.indicators)}
                                      :subscriptions {:indicators/model (. stock
                                                                           :subscriptions
                                                                           :indicators/model)}
                                      :validators {:indicators (. stock
                                                                  :validators
                                                                  :indicators)}})
        :misa.ui.status.render (let [stock (require :misa.standard.presentation.elements)]
                                 {:components {:default.status.indicators (. stock
                                                                             :components
                                                                             :default.status.indicators)}
                                  :requirements {:component.status (. stock
                                                                      :requirements
                                                                      :component.status)}
                                  :value-renderers {:activity (. stock
                                                                 :value-renderers
                                                                 :activity)}})
        :misa.ui.themes (let [stock (require :misa.standard.presentation.theme)]
                          {:events {:themes/app/start (. stock :events
                                                         :themes/app/start)
                                    :themes/themes/loaded (. stock :events
                                                             :themes/themes/loaded)
                                    :themes/themes/swap (. stock :events
                                                           :themes/themes/swap)}
                           :services {:themes.lookup (. stock :services
                                                        :themes.lookup)
                                      :themes.swap (. stock :services
                                                      :themes.swap)
                                      :themes.style (. stock :services
                                                       :themes.style)}
                           :validators {:themes (. stock :validators :themes)}})
        :misa.ui.themes.default (let [stock (require :misa.standard.presentation.theme)]
                                  {:themes {:default (. stock :themes :default)
                                            :light (. stock :themes :light)}})
        :misa.ui.values (let [stock (require :misa.standard.presentation.values)]
                          {:services {:values.render (. stock :services
                                                        :values.render)
                                      :values.timestamp->seconds (. stock
                                                                    :services
                                                                    :values.timestamp->seconds)}
                           :validators {:value-renderers (. stock :validators
                                                            :value-renderers)}
                           :value-renderers {:tokens (. stock :value-renderers
                                                        :tokens)
                                             :ratio (. stock :value-renderers
                                                       :ratio)
                                             :percent (. stock :value-renderers
                                                         :percent)
                                             :number (. stock :value-renderers
                                                        :number)
                                             :datetime (. stock
                                                          :value-renderers
                                                          :datetime)
                                             :duration (. stock
                                                          :value-renderers
                                                          :duration)
                                             :rate (. stock :value-renderers
                                                      :rate)
                                             :money (. stock :value-renderers
                                                       :money)
                                             :text (. stock :value-renderers
                                                      :text)
                                             :unavailable (. stock
                                                             :value-renderers
                                                             :unavailable)
                                             :boolean (. stock :value-renderers
                                                         :boolean)
                                             :sequence (. stock
                                                          :value-renderers
                                                          :sequence)
                                             :timestamp (. stock
                                                           :value-renderers
                                                           :timestamp)}})
        :misa.usage (require :misa.standard.usage)
        :misa.usage.dialog (require :misa.standard.usage.dialog)})

(local result (misa.snapshot catalogs))
(each [_ catalog (pairs result)]
  (each [_ event (pairs (or catalog.events {}))]
    (set event.priority nil)))

result
