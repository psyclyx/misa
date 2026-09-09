;; A broken extension is a local presentation failure, including cached views.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:config {:themes {:persist false} :components {:persist false}}})
(each [_ name (ipairs [:json :themes :theme/default :components])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))
(var calls 0)
(misa._setup_effects
 {:fx [{:type :register/component :id :default.fixture
        :value {:render (fn [model]
                          (set calls (+ calls 1))
                          (if model.throw (error "fixture exploded") model.view))}}
       {:type :register/component :id :default.parent
        :value {:compose true :render (fn [model context]
                                        (local child (context.render_child :fixture model))
                                        {:lines [{:spans [{:text :before}]}
                                                 (. child.lines 1)
                                                 {:spans [{:text :after}]}]})}}]})
(local db {:themes {:active :default} :components {:roles {}}})
(local render-context {:columns 32})
(fn broken [model detail]
  (local view (misa.render_component db :fixture model render-context))
  (assert (= (length view.lines) 1))
  (assert (<= (length (. view.lines 1 :spans 1 :text)) 32))
  (assert (= view.component_error.role :fixture))
  (assert (view.component_error.detail:find detail 1 true))
  view)
(broken {:throw true} "fixture exploded")
(broken {:view false} "return a table")
(broken {:view {:lines :bad}} "lines must be an array")
(broken {:view {:lines [{:spans [{:text 4}]}]}} "span text")
(broken {:view {:lines [{:spans [{:text "bad\nline"}]}]}} "span text")
(broken {:view {:lines [{:spans [{:text :ok :animation {:frames [false]}}]}]}} "animation frame")
(broken {:view {:lines [{:spans [{:text :ok}]}] :cursor {:row 2 :byte 0}}} "cursor row")
(broken {:view {:lines [{:spans [{:text :ok}]}] :cursor {:row 1 :byte 3}}} "cursor byte")
(broken {:view {:lines [{:spans [{:text "é"}]}] :cursor {:row 1 :byte 1}}} "UTF-8")
(broken {:view {:lines [{:spans [] :image {:id 1 :width -1}}]}} "image geometry")
(local nested (misa.render_component db :parent {:throw true} render-context))
(assert (= (. nested.lines 1 :spans 1 :text) :before))
(assert (= (. nested.lines 2 :spans 1 :text) "Component fixture failed"))
(assert (= (. nested.lines 3 :spans 1 :text) :after))
(local collection [{:id :bad :role :fixture :model {:throw true}}
                   {:id :good :role :fixture :model {:view {:lines [{:spans [{:text :healthy}]}]}}}])
(local first (misa.project_components db :failures collection render-context))
(assert (. first.views 1 :component_error))
(assert (= (. first.views 2 :lines 1 :spans 1 :text) :healthy))
(local before calls)
(local cached (misa.project_components db :failures collection render-context))
(assert (= calls before) "cached failed component was retried on unchanged projection")
(local recovered (misa.project_components db :failures
                                         [{:id :bad :role :fixture :model {:view {:lines [{:spans [{:text :recovered}]}]}}}]
                                         render-context))
(assert (= (. recovered.views 1 :component_error) nil))
(assert (= (. recovered.views 1 :lines 1 :spans 1 :text) :recovered))
(local missing (misa.project_components db :missing
                                       [{:id :missing :role :unregistered :model {}}]
                                       render-context))
(assert (= (. missing.views 1 :component_error :role) :unregistered))
(output "component failure containment passed\n")
