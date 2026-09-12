(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:argv [] :config {:request_options {:enabled false}}})
(local one {:id :one :provider :test :model :one
            :api {:request_options {:enabled {:choices [false true] :default true}}}})
(local two {:id :two :provider :test :model :two
            :api {:request_options {:region {:choices [:west :east] :default :east}}}})
(set misa.models.all (fn [] [one two]))
;; Register the consumer first: startup and subsequent selections must use the
;; committed model, not depend on the ordering of extension registrations.
(local app ((require :tests.application) {:argv [] :config {}}))
(local declarations (require :tests.declarations))
(each [_ name (ipairs [:misa.json :misa.models.options :misa.models])]
  (local specs (. (require :tests.stock) name))

  (app.define specs))
(var observed nil)
(app.define (declarations.collect :model-startup-1 [{:catalog :events  :value {:event :test/read :handler (fn [db] (set observed db) nil)}}]))
(app.install context)
(fn dispatch [event]
  (local pending [event])
  (var at 1)
  (while (. pending at)
    (local effects (misa._dispatch (. pending at) {:columns 80 :lines 24 :interactive false}
                                  {:wall_ms 0 :monotonic_ms 0}))
    (misa._commit)
    (each [_ effect (ipairs effects)]
      (when (= effect.type :dispatch) (table.insert pending effect.event)))
    (set at (+ at 1))
    (assert (< at 100) "startup dispatch did not settle")))
(fn read-state [] (dispatch {:type :test/read}) observed)
(dispatch {:type :app/start})
(local initial (read-state))
(assert (= initial.models.selected :one))
(assert (= initial.request_options.model_id :one))
(assert (= initial.request_options.values.enabled false))
;; The load of the persisted selection is pending until it reports back.
(assert (= initial.models.selection_pending true))
(dispatch {:type :model/select :id :two})
(local selected (read-state))
(assert (= selected.request_options.model_id :two))
(assert (= selected.request_options.values.region :east))
(assert (= selected.request_options.values.enabled nil))
(assert (= initial.request_options.model_id :one) "selection mutated previous state")
(dispatch {:type :models/provider-availability :provider :test :available false})
(local unavailable (read-state))
(assert (= unavailable.models.selected nil))
(assert (= unavailable.request_options.model_id nil))
(assert (= (next unavailable.request_options.values) nil))
;; Availability returning restores the model the user chose, not the catalogue's
;; first entry.
(dispatch {:type :models/provider-availability :provider :test :available true})
(local restored (read-state))
(assert (= restored.models.selected :two))
(assert (= restored.request_options.model_id :two))
(assert (= restored.request_options.values.region :east))
;; A persisted selection replaces the configured default once its load settles.
(dispatch {:type :model/selection-loaded
           :found true
           :data {:selected :one}})
(local persisted (read-state))
(assert (= persisted.models.selected :one))
(assert (= persisted.models.preferred :one))
(assert (= persisted.models.selection_pending false))
(assert (= persisted.request_options.model_id :one))
;; The switched model's own default replaces the option of the previous model.
(assert (= persisted.request_options.values.enabled true))
;; A persisted model the catalogue does not offer yet stays preferred instead of
;; falling back to the default, and is selected when its provider arrives.
(dispatch {:type :model/selection-loaded
           :found true
           :data {:selected :late/one}})
(local waiting (read-state))
(assert (= waiting.models.selected nil) "an unoffered saved model was selected")
(assert (= waiting.models.preferred :late/one))
(assert (= waiting.models.selection_pending false))
(dispatch {:type :models/replace-provider
           :provider :late
           :models [{:context_window 10 :id :late/one :model :late-one}]})
(local arrived (read-state))
(assert (= arrived.models.selected :late/one) "discovered saved model was not restored")
(assert (= arrived.request_options.model_id :late/one))
(assert (= (. (read-state) :models :selected) :late/one))
(output "model startup ordering contracts passed\n")
