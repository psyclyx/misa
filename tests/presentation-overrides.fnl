(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local stock (require :tests.stock))
(local configuration
       {:status {:indicators [:activity
                              {:id :plan :priority 1}
                              {:id :extra :priority 200}]}})

(local app ((require :tests.application) {:argv [] :config configuration}))
(local definitions (require :tests.declarations))

;; Replacing one implementation is ordinary table composition before installation.
(local value-stock (. stock :misa.ui.values))
(local renderers (collect [id render (pairs value-stock.value-renderers)] id
                   render))
(tset renderers :tokens
      (fn [fact]
        [{:text (.. fact.value " tokens")}]))
(app.define {:services value-stock.services
             :validators value-stock.validators
             :value-renderers renderers})
(local tool-stock (. stock :misa.transcript.tools))
(local adapters (collect [id adapter (pairs tool-stock.tool-presentations)] id
                  adapter))
(tset adapters :shell
      (fn [model]
        {:result {:role :content.text
                  :model {:text (.. "custom: " model.result)}}}))
(app.define {:services tool-stock.services
             :validators tool-stock.validators
             :tool-presentations adapters})
(app.define (. stock :misa.ui.status.indicators))
(each [_ id (ipairs [:activity :session :plan :extra])]
  (app.define (definitions.collect :fixture
                [{:catalog :subscriptions
                  :id (.. :fixture/ id)
                  :value {:id (.. :fixture/ id)
                          :inputs []
                          :compute (fn [] {:type :text :value id})}}
                 {:catalog :indicators
                  :id id
                  :value {:id id :query [(.. :fixture/ id)]}}])))

(app.install)
(assert (= (. (misa.values.render {:type :tokens :value 12}) 1 :text)
           "12 tokens"))
(assert (= (. (misa.values.render {:type :boolean :value false}) 1 :text)
           "false"))
(assert (= (. (misa.tools.presentation {:name :shell :result :done}) :result
              :model :text) "custom: done"))
(assert (= (. (misa.tools.presentation {:name :read_file :result :contents})
              :result :role) :content.code))
(local indicators (. (misa.sub {} [:indicators/model]) :indicators))
(assert (= (length indicators) 3))
(assert (= (. indicators 1 :id) :activity))
(assert (= (. indicators 2 :id) :plan))
(assert (= (. indicators 2 :priority) 1))
(assert (= (. indicators 3 :id) :extra))
(assert (= (. indicators 3 :priority) 200))
(output "presentation overrides passed\n")
