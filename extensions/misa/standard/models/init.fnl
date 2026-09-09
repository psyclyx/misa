(local {: complete-model-open
        : complete-model-role
        : compute-models-indicator
        : compute-models-projection
        : compute-models-selected
        : models-for-role
        : on-model-open
        : on-model-picker-open
        : on-model-role
        : on-model-select
        : selected-model-open
        : on-app-start
        : on-models-provider-availability
        : on-models-update
        : on-models-replace-provider
        : on-model-roles-loaded} (require :misa.models))

{:actions {:models.open {:binding {:action :open_model_picker :context :global}
                         :event {:type :model/picker-open}
                         :id :models.open
                         :label "Choose active model"}}
 :commands {:/model {:choice_purpose :models
                     :complete complete-model-open
                     :description "Choose the active model"
                     :event :model/open
                     :name :/model
                     :preference_scope :models
                     :selected selected-model-open}
            :/role {:name :/role
                    :description "Assign a model to a role (default or summarizer)"
                    :event :model/role
                    :complete complete-model-role}}
 :events {"models/app/start" {:event :app/start
                              :handler (fn [db event cofx]
                                         (on-app-start cofx.config db event
                                                       cofx))
                              :priority 58000}
          "models/model/picker-open" {:event :model/picker-open
                                      :handler on-model-picker-open
                                      :priority 58000}
          "models/model/open" {:event :model/open
                               :handler on-model-open
                               :priority 58000}
          "models/models/provider-availability" {:event :models/provider-availability
                                                 :handler (fn [db event cofx]
                                                            (on-models-provider-availability cofx.config
                                                                                             db
                                                                                             event))
                                                 :priority 58000}
          "models/models/update" {:event :models/update
                                  :handler (fn [db event cofx]
                                             (on-models-update cofx.config db
                                                               event))
                                  :priority 58000}
          "models/models/replace-provider" {:event :models/replace-provider
                                            :handler (fn [db event cofx]
                                                       (on-models-replace-provider cofx.config
                                                                                   db
                                                                                   event))
                                            :priority 58000}
          "models/model/select" {:event :model/select
                                 :handler on-model-select
                                 :priority 58000}
          "models/model/role" {:event :model/role
                               :handler on-model-role
                               :priority 58000}
          "models/model/roles-loaded" {:event :model/roles-loaded
                                       :handler (fn [db event cofx]
                                                  (on-model-roles-loaded cofx.config
                                                                         db
                                                                         event))
                                       :priority 58000}}
 :indicators {:model {:icon "◆"
                      :id :model
                      :label :model
                      :hotkey {:action :open_model_picker :context :global}
                      :query [:models/indicator]}}
 :keybindings {"global/open_model_picker" {:action :open_model_picker
                                           :context :global
                                           :default [:alt+m]}}
 :services {:models.selected (fn [db] "Return the currently selected model."
                               (misa.sub db [:models/selected]))
            :models.state (fn [db]
                            "Return the current model catalogue and selection state."
                            (misa.sub db [:models/projection]))
            :models.for-role models-for-role}
 :subscriptions {:models/indicator {:id :models/indicator
                                    :inputs [[:models/selected]]
                                    :compute compute-models-indicator}
                 :models/selected {:id :models/selected
                                   :inputs [[:db/path :models :entries]
                                            [:db/path :models :selected]]
                                   :compute compute-models-selected}
                 :models/projection {:id :models/projection
                                     :inputs [[:db/path
                                               :models
                                               :configured_default]
                                              [:models/selected]]
                                     :compute compute-models-projection}}}
