;; Registry and projection for small semantic status values. Features declare

;; values; configuration chooses presentation and order. No feature knows where

;; or how the resulting row is composed.

{:setup (fn [context]
          (var (definitions sealed) (values {} false))
          (var root (or (and (= (type context.config) :table)
                             context.config.status) nil))
          (set root (or (and (= (type root) :table) root) {}))
          (local configured
                 (or root.indicators
                     [{:id :activity :priority 100 :representation :icon}
                      {:id :model :priority 90 :representation :label}
                      {:hotkey true
                       :id :effort
                       :priority 70
                       :representation :label}
                      {:id :session :priority 40 :representation :icon}
                      {:id :context :priority 80 :representation :label}
                      {:hotkey true
                       :id :transcript-detail
                       :priority 20
                       :representation :label}]))
          (assert (= (type configured) :table)
                  "config.status.indicators must be an array")

          (fn misa.reg_indicator [definition]
            (assert (not sealed) "indicator registrations are sealed")
            (assert (and (and (= (type definition) :table)
                              (= (type definition.id) :string))
                         (not= definition.id ""))
                    "indicator ID must be nonempty")
            (assert (= (type definition.value) :function)
                    "indicator must provide a value projection")
            (assert (or (= definition.label nil)
                        (= (type definition.label) :string))
                    "indicator label must be a string")
            (assert (or (= definition.icon nil)
                        (= (type definition.icon) :string))
                    "indicator icon must be a string")
            (assert (or (= definition.hotkey nil)
                        (and (and (= (type definition.hotkey) :table)
                                  (= (type definition.hotkey.context) :string))
                             (= (type definition.hotkey.action) :string)))
                    "indicator hotkey must name a context and action")
            (assert (= (. definitions definition.id) nil)
                    (.. "duplicate indicator: " definition.id))
            (tset definitions definition.id definition)
            nil)

          (misa.reg_interceptor {:before (fn [tx]
                                           (when (= tx.event.type :app/start)
                                             (set sealed true))
                                           tx)
                                 :id :indicators/seal})

          (fn misa.indicators_projection [db render-context]
            (local ___values___ {})
            (each [index selection (ipairs configured)]
              (when (= (type selection) :string)
                (set-forcibly! selection {:id selection}))
              (assert (and (= (type selection) :table)
                           (= (type selection.id) :string))
                      "invalid status indicator selection")
              (local definition (. definitions selection.id))
              (when definition
                (local value (definition.value db))
                (when (and (and (not= value nil) (not= value false))
                           (not= (tostring value) ""))
                  (local representation (or selection.representation :label))
                  (assert (or (= representation :label)
                              (= representation :icon))
                          "indicator representation must be label or icon")
                  (local label
                         (or (and (= representation :icon)
                                  (or (or definition.icon definition.label)
                                      selection.id))
                             (or definition.label selection.id)))
                  (var hotkey nil)
                  (if (and (and (= selection.hotkey true) definition.hotkey)
                           misa.keybinding_hint)
                      (set hotkey
                           (misa.keybinding_hint definition.hotkey.context
                                                 definition.hotkey.action))
                      (= (type selection.hotkey) :string)
                      (set hotkey selection.hotkey))
                  (var action definition.action)
                  (when (and (and (not action) definition.hotkey) misa.actions)
                    (each [_ candidate (ipairs (misa.actions))]
                      (when (and (and candidate.binding
                                      (= candidate.binding.context
                                         definition.hotkey.context))
                                 (= candidate.binding.action
                                    definition.hotkey.action))
                        (set action candidate.id)
                        (lua :break))))
                  (tset ___values___ (+ (length ___values___) 1)
                        {: action
                         : hotkey
                         :id selection.id
                         : label
                         :priority (or (tonumber selection.priority)
                                       (- 1000 index))
                         :value (tostring value)}))))
            (local context-copy {})
            (each [key value (pairs (or render-context {}))]
              (tset context-copy key value))
            (. (misa.render_component db :status.indicators
                                      {:indicators ___values___} context-copy)
               :lines))

          nil)}

