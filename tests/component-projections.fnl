(local fennel (require :fennel))
(local output print)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(each [_ name (ipairs [:json :themes :theme/default :components :syntax :costs])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) {:config {}}))
(var renders 0)
(var hints 0)
(misa._setup_effects
 {:fx [{:type :register/component :id :default.fixture
        :value {:render (fn [model _ previous]
                          (set renders (+ renders 1))
                          (when previous (set hints (+ hints 1)))
                          (values {:lines [{:spans [{:text model.text :style :plain
                                                    :action model.action
                                                    :link (.. "https://test/" model.text)}]}]}
                                  {:text model.text}))}}
       {:type :register/component :id :alternate
        :value {:render (fn [model] {:lines [{:spans [{:text (.. :alternate- model.text)}]}]})}}
       {:type :register/theme :id :other
        :value {:palette {:ink :red} :styles {:plain {:foreground :ink}}}}]})
(local db {:components {:roles {}} :themes {:active :default}})
(local context {:columns 80})
(local items (fcollect [i 1 300] {:id (tostring i) :role :fixture :model {:text (tostring i)}}))
(fn project [state records ctx] (misa.project_components state :test records (or ctx context)))
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
  (icollect [_ item (ipairs items)]
    {:id item.id :role item.role
     :model {:text item.model.text
             :syntax (misa.syntax_projection enriched-db {:id item.id :text item.model.text})
             :cost (misa.response_cost_projection enriched-db item.id)}}))
(local enriched (misa.project_components enriched-db :enriched (enrich) context))
(local before-enriched renders)
(local enriched-again (misa.project_components enriched-db :enriched (enrich) context))
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
(local themed (project (misa.swap_theme db :other) changed))
(assert (= renders 301) "theme switch recomputed semantic output")
(assert (= (. themed.views 1 :lines 1 :spans 1 :style :foreground) :red))
(project db changed {:columns 40})
(assert (= renders 601) "layout context did not invalidate component output")
(local removed (project db []))
(assert (= (next removed.entries) nil) "removed items retained cache entries")
(assert (not (pcall project db [(. items 1) (. items 1)])) "duplicate ids were accepted")
(local button [{:id :button :role :fixture :model {:text :button :action :click}}])
(local resting (misa.project_components db :button button context))
(local link-only (misa.project_components (misa.patch db {:hover_link "https://test/button"}) :button button context))
(assert (= (. resting.views 1) (. link-only.views 1)) "link hover overrode action precedence")
(local action-hover (misa.project_components (misa.patch db {:hover_action :click}) :button button context))
(assert (not= (. resting.views 1) (. action-hover.views 1)))
(local swapped (misa.project_components (misa.swap_component db :fixture :alternate) :button button context))
(assert (= (. swapped.views 1 :lines 1 :spans 1 :text) :alternate-button))
(assert (not (pcall misa.project_components db nil button context)) "missing collection owner was accepted")

;; The framework owns the collection cache: speculative output never replaces
;; the committed entries, including component-owned incremental hints.
(var observed nil)
(misa._setup_effects
 {:fx [{:type :register/event :name :set
        :handler (fn [_ event] {:patch {:components (misa.replace db.components) :themes db.themes
                                      :text event.text :fail (= event.fail true)}})}
       {:type :register/view
        :handler (fn [state]
                   (set observed (misa.project_components state :transaction
                                                         [{:id :one :role :fixture :model {:text state.text}}] context))
                   (assert (not state.fail) "reject view")
                   {:lines []})}]})
(misa._seal {:argv [] :config {}})
(fn dispatch [text fail]
  (misa._dispatch {:type :set : text : fail} {:columns 80 :lines 24 :interactive true}
                  {:wall_ms 0 :monotonic_ms 0}))
(dispatch :committed)
(misa._commit)
(local committed observed)
(dispatch :speculative)
(misa._rollback)
(dispatch :committed)
(assert (= (. observed.views 1) (. committed.views 1)) "rollback lost committed render identity")
(misa._commit)
(assert (not (pcall dispatch :failed true)))
(dispatch :committed)
(assert (= (. observed.views 1) (. committed.views 1)) "failed view replaced committed output")
(misa._commit)
(output "component projection contracts passed")
