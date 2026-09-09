(local fennel (require :fennel))
(local output print)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(each [_ name (ipairs [:misa.json :misa.ui.themes :misa.ui.themes.default :misa.ui.components :misa.transcript.syntax :misa.costs])]
  (app.define ((. (require name) :build) {:config {}})))
(var renders 0)
(var hints 0)

(local db {:components {:roles {}} :themes {:active :default}})
(local context {:columns 80})
(local items (fcollect [i 1 300] {:id (tostring i) :role :fixture :model {:text (tostring i)}}))
(fn project [state records ctx] (misa.components.project state :test records (or ctx context)))
(app.define (definitions.build :fixture [{:catalog :components :id :default.fixture :value {:render (fn [model _ previous]
                          (set renders (+ renders 1))
                          (when previous (set hints (+ hints 1)))
                          (values {:lines [{:spans [{:text model.text :style :plain
                                                    :action model.action
                                                    :link (.. "https://test/" model.text)}]}]}
                                  {:text model.text}))}}
       {:catalog :components :id :alternate :value {:render (fn [model] {:lines [{:spans [{:text (.. :alternate- model.text)}]}]})}}
       {:catalog :themes :id :other :value {:palette {:ink :red} :styles {:plain {:foreground :ink}}}}]))
(var observed nil)
(app.define (definitions.build :fixture [{:catalog :events  :value {:event :set :handler (fn [_ event] {:patch {:components (misa.replace db.components) :themes db.themes
                                      :text event.text :fail (= event.fail true)}})}}
       {:catalog :views :id :main :value (fn [state]
                   (set observed (misa.components.project state :transaction
                                                         [{:id :one :role :fixture :model {:text state.text}}] context))
                   (assert (not state.fail) "reject view")
                   {:lines []})}]))
(app.install)
(local first (project db items))
(assert (= renders 300))
(local repeated (project {:components db.components :themes db.themes :unrelated true} items))
(assert (= renders 300) "unchanged collection rendered again or thrashed its scope")
(for [i 1 300] (assert (= (. repeated.views i) (. first.views i))))
;; Bundled per-item enrichment must not fill the flat scope with 600 queries
;; and evict the render collection before the next frame.
(local documents {})
(local responses {})
(for [i 1 300]
  (tset documents (.. "0:" i) {:source (tostring i) :slots [] :document {:id i}})
  (tset responses (tostring i) {:cost {:usd i}}))
(local enriched-db (misa.patch db {:syntax {:documents (misa.replace documents)}
                                  :costs {:responses (misa.replace responses)}}))
(fn enrich []
  (local syntax (misa.syntax.all enriched-db))
  (icollect [_ item (ipairs items)]
    {:id item.id :role item.role
     :model {:text item.model.text
             :syntax (misa.syntax.for-model syntax {:id item.id :text item.model.text})
             :cost (misa.costs.response enriched-db item.id)}}))
(local enriched (misa.components.project enriched-db :enriched (enrich) context))
(local before-enriched renders)
(local enriched-again (misa.components.project enriched-db :enriched (enrich) context))
(assert (= renders before-enriched) "syntax/cost queries evicted collection memoization")
(assert (= (. enriched.views 300) (. enriched-again.views 300)))
(set renders 300)
(set hints 0)
(local changed (fcollect [i 1 300]
                 (if (= i 300) {:id "300" :role :fixture :model {:text :changed}} (. items i))))
(local updated (project db changed))
(assert (= renders 301))
(assert (= hints 1) "changed component lost its immutable projection hint")
(assert (= (. first.views 300 :lines 1 :spans 1 :text) "300"))
(for [i 1 299] (assert (= (. updated.views i) (. first.views i))))
(local hovered (project (misa.patch db {:hover_link "https://test/changed"}) changed))
(assert (= renders 301) "hover recomputed semantic component output")
(assert (not= (. hovered.views 300) (. updated.views 300)))
(for [i 1 299] (assert (= (. hovered.views i) (. updated.views i))))
(local left (project db changed))
(assert (= renders 301))
(assert (= (. left.views 300 :lines 1 :spans 1 :style :background) nil))
(local themed (project (misa.themes.swap db :other) changed))
(assert (= renders 301) "theme switch recomputed semantic output")
(assert (= (. themed.views 1 :lines 1 :spans 1 :style :foreground) :red))
(project db changed {:columns 40})
(assert (= renders 601) "layout context did not invalidate component output")
(local removed (project db []))
(assert (= (next removed.entries) nil) "removed items retained cache entries")
(assert (not (pcall project db [(. items 1) (. items 1)])) "duplicate ids were accepted")
(local button [{:id :button :role :fixture :model {:text :button :action :click}}])
(local resting (misa.components.project db :button button context))
(local link-only (misa.components.project (misa.patch db {:hover_link "https://test/button"}) :button button context))
(assert (= (. resting.views 1) (. link-only.views 1)) "link hover overrode action precedence")
(local action-hover (misa.components.project (misa.patch db {:hover_action :click}) :button button context))
(assert (not= (. resting.views 1) (. action-hover.views 1)))
(local swapped (misa.components.project (misa.components.swap db :fixture :alternate) :button button context))
(assert (= (. swapped.views 1 :lines 1 :spans 1 :text) :alternate-button))
(assert (not (pcall misa.components.project db nil button context)) "missing collection owner was accepted")

;; The framework owns the collection cache: speculative output never replaces
;; the committed entries, including component-owned incremental hints.



(fn dispatch [text fail]
  (misa._dispatch {:type :set : text : fail} {:columns 80 :lines 24 :interactive true}
                  {:wall_ms 0 :monotonic_ms 0})
  (misa._commit))
(fn project-frame []
  (misa._project {:columns 80 :lines 24 :interactive true} {:wall_ms 0 :monotonic_ms 0}))
(dispatch :committed)
(project-frame)
(misa._commit_projection)
(local committed observed)
(dispatch :speculative)
(project-frame)
(misa._rollback_projection)
(dispatch :committed)
(project-frame)
(assert (= (. observed.views 1) (. committed.views 1)) "rollback lost committed render identity")
(misa._commit_projection)
(dispatch :failed true)
(assert (not (pcall project-frame)))
(misa._rollback_projection)
(dispatch :committed)
(project-frame)
(assert (= (. observed.views 1) (. committed.views 1)) "failed view replaced committed output")
(misa._commit_projection)
(output "component projection contracts passed")
