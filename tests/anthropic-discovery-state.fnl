(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local declarations (require :tests.declarations))
(each [_ name (ipairs [:misa.json :misa.protocols.anthropic])]
  (app.include (. (require :tests.stock) name) {:config {}}))
(local protocol (require :misa.protocols.anthropic))
(local transport {:id :test :url "https://example.invalid/messages"
                                      :models_url "https://example.invalid/models?region=test"
                                      :models [] :credential :test :catalogue_authoritative true
                                      :model_filter (fn [model] (not= model.id :skip))})
(local specs {:events {:test/discover {:event :models/discover :handler (fn [db event] (protocol.discover-models transport db event))}
 :test/models {:event :provider/test-models :handler (fn [db event] (protocol.page transport :anthropic.messages.test db event))}}})
(local handlers {})
(each [_ spec (pairs specs.events)]
  (tset handlers spec.event spec.handler))
(app.install)
(fn transition [db event]
  (local (before input) (values (misa.json.encode db) (misa.json.encode event)))
  (local result ((. handlers event.type) db event))
  (assert (= before (misa.json.encode db)) "discovery mutated previous state")
  (assert (= input (misa.json.encode event)) "discovery mutated a page")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (or (and result result.fx) [])))
(local original {:model_discovery {:other [{:id :unrelated}]}})
(local (initial request) (transition original {:type :models/discover :provider :test}))
(assert (= original.model_discovery.test nil))
(assert (= initial.model_discovery.other original.model_discovery.other))
(assert (= (. request 1 :url) "https://example.invalid/models?region=test&limit=1000"))
(local failure
       (G.for_all (G.vector (G.elements [{:id :beta :display_name :Beta :context_length 100}
                                         {:id :alpha :max_input_tokens 200
                                          :request_options {:effort {:choices [:low :high]}}}
                                         {:id :skip} {:id :gamma}]))
                  (fn [items]
                    (local (whole result) (transition initial {:type :provider/test-models :ok true
                                                               :data {:data items :has_more false}}))
                    (var split initial)
                    (var final nil)
                    (each [index item (ipairs items)]
                      (local (next fx) (transition split {:type :provider/test-models :ok true
                                                         :data {:data [item] :last_id (.. :page- index)
                                                                :has_more (< index (length items))}}))
                      (set split next)
                      (set final fx))
                    (when (= (length items) 0)
                      (local (next fx) (transition split {:type :provider/test-models :ok true :data {:data []}}))
                      (set split next)
                      (set final fx))
                    (assert (= (misa.json.encode whole) (misa.json.encode split)))
                    (assert (= (misa.json.encode result) (misa.json.encode final)) "pagination changed the catalogue"))
                  {:cases 1000 :size 20}))
(assert (not failure) (and failure (fennel.view failure)))
(local (first next-request) (transition initial {:type :provider/test-models :ok true
                                                :data {:data [{:id :beta}] :has_more true :last_id :cursor}}))
(assert (= (. next-request 1 :url) "https://example.invalid/models?region=test&limit=1000&after_id=cursor"))
(local (done completed) (transition first {:type :provider/test-models :ok true :data {:data [{:id :alpha}]}}))
(assert (= (. completed 1 :event :models 1 :id) :test/alpha))
(assert (= (. first.model_discovery.test 1 :id) :test/beta))
(assert (= done.model_discovery.test nil))
(local (failed failure-fx) (transition first {:type :provider/test-models :ok false}))
(assert (= failed.model_discovery.test nil))
(assert (= (length failure-fx) 1))
(assert (= (. failure-fx 1 :event :type) :models/discovery-complete))
(local (empty empty-fx) (transition initial {:type :provider/test-models :ok true :data {:data []}}))
(assert (= (length (. empty-fx 1 :event :models)) 0))
(assert (= (. empty-fx 1 :event :authoritative) true))
(local (invalid invalid-fx) (transition first {:type :provider/test-models :ok true
                                              :data {:data [{:id :new}] :has_more true}}))
(assert (= invalid.model_discovery.test nil))
(assert (= (length invalid-fx) 1))
(assert (= (transition initial {:type :models/discover :provider :other}) initial))
(output "Anthropic discovery state properties passed\n")
