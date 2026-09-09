(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local context {:config {:values {:roles {:tokens :custom.tokens}}
                        :tool_presentations {:roles {:shell :custom.shell}}
                        :status {:indicator_overrides {:session false :plan {:priority 1} :extra {:priority 200}}}}})
(each [_ name (ipairs [:values :tool/presentations :indicators])]
  (app.define ((fennel.dofile (.. :extensions/ name :.fnl)) context)))

(app.define (definitions :fixture [{:catalog :value-renderers :id :custom.tokens :value (fn [fact] [{:text (.. fact.value " tokens")}])}
       (let [definition (fn [model] {:result {:role :content.text :model {:text (.. "custom: " model.result)}}})] {:catalog :tool-presentations :id :custom.shell :value definition})]))
(each [_ id (ipairs [:activity :session :plan :extra])]
  (app.define (definitions :fixture [(let [definition {:id (.. :fixture/ id) :inputs []
                                                       :compute (fn [] {:type :text :value id})}] {:catalog :subscriptions :id (. definition :id) :value definition})
                            (let [definition {: id :query [(.. :fixture/ id)]}] {:catalog :indicators :id (. definition :id) :value definition})])))
(app.install)
(assert (= (. (misa.values.render {:type :tokens :value 12}) 1 :text) "12 tokens"))
(assert (= (. (misa.values.render {:type :boolean :value false}) 1 :text) "false"))
(assert (= (. (misa.tools.presentation {:name :shell :result :done}) :result :model :text) "custom: done"))
(assert (= (. (misa.tools.presentation {:name :read_file :result :contents}) :result :role) :content.code))

(local indicators (. (misa.sub {} [:indicators/model]) :indicators))
(assert (= (length indicators) 3))
(assert (= (. indicators 1 :id) :activity) "override discarded unchanged default")
(assert (= (. indicators 2 :id) :plan))
(assert (= (. indicators 2 :priority) 1))
(assert (= (. indicators 3 :id) :extra))
(assert (= (. indicators 3 :priority) 200))
(output "presentation overrides passed\n")
