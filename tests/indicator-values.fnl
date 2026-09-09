(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local context {:argv [] :config {:themes {:persist false} :components {:persist false}
                                  :animations {:persist false}
                                  :status {:indicators [:zero :false :hidden :custom :activity]}}})
(each [_ name (ipairs [:misa.json :misa.keybindings :misa.ui.layout :misa.ui.themes :misa.ui.themes.default :misa.ui.components :misa.ui.values :misa.ui.status.render
                       :misa.ui.animations :misa.ui.animations.default :misa.ui.status.indicators])]
  (app.define ((. (require name) :build) context)))
(local calls {})
(each [_ id (ipairs [:zero :false :hidden :custom :activity])]
  ;; Deliberately register the consumer before its query.
  (app.define (definitions.build :fixture [(let [definition {: id :query [(.. :test/ id)]}] {:catalog :indicators :id (. definition :id) :value definition})
                            (let [definition {:id (.. :test/ id) :inputs [[:db/path :facts id]]
                                     :compute (fn [inputs]
                                                (tset calls id (+ (or (. calls id) 0) 1))
                                                (. inputs 1))}] {:catalog :subscriptions :id (. definition :id) :value definition})])))
(local shared [{:text "extension"}])
(app.define (definitions.build :fixture [{:catalog :animations :id :test.still :value {:frames ["Z"]}}
                          {:catalog :value-renderers :id :custom :value (fn [fact] (assert (= fact.value 7)) shared)}]))
(each [_ malformed (ipairs [{:id :legacy :value (fn [] :text)}
                            {:id :empty :query []}
                            {:id :invalid :query [false]}])]
  (assert (not (pcall app.definitions.validators.indicators malformed.id malformed))))
(assert (not (pcall app.define {:indicators {:zero {:id :zero :query [:test/zero]}}})))
(assert (not (pcall app.define {:value-renderers {:text (fn [])}})))
(var observed nil)
(app.define (definitions.build :fixture [{:catalog :events  :value {:event :test/read :handler (fn [db] (set observed db) nil)}}]))
(app.install)
(each [_ event (ipairs [{:type :app/start} {:type :test/read}])]
  (misa._dispatch event {:interactive true :columns 100 :lines 24} {:wall_ms 0 :monotonic_ms 0})
  (misa._commit))
(local db (misa.patch observed {:facts {:zero {:type :tokens :value 0}
                                       :false {:type :boolean :value false}
                                       :custom {:type :custom :value 7}
                                       :activity {:type :activity :state :thinking}}}))
(local model (misa.sub db [:indicators/model]))
(assert (= (length model.indicators) 4) "nil/zero/false visibility is wrong")
(assert (= (. model.indicators 1 :fact :value) 0))
(assert (= (. model.indicators 2 :fact :value) false))
(local counts (misa.json.encode calls))
(local unrelated (misa.patch db {:hover_action :unrelated :agent {:status :working}}))
(assert (= (misa.sub unrelated [:indicators/model]) model))
(assert (= counts (misa.json.encode calls)) "unrelated state recomputed facts")
(local altered (misa.patch db {:facts {:zero {:value 1234}}}))
(local changed (misa.sub altered [:indicators/model]))
(assert (= (. changed.indicators 2 :fact) (. model.indicators 2 :fact)))
(assert (= (. model.indicators 1 :fact :value) 0) "projection mutated previous facts")
(assert (= calls.zero 2))
(assert (= calls.false 1))
(fn text [spans] (table.concat (icollect [_ span (ipairs spans)] span.text)))
(assert (= (text (misa.values.render {:type :tokens :value 1234})) "1.2k"))
(assert (= (text (misa.values.render {:type :ratio :used 1234 :unit :tokens})) "1.2k/?"))
(assert (= (text (misa.values.render {:type :percent :value 0 :basis :remaining})) "0% left"))
(assert (= (text (misa.values.render {:type :boolean :value false})) "false"))
(assert (= (text (misa.values.render {:type :money :amount 0 :currency :USD})) "$0"))
(assert (= (text (misa.values.render {:type :unavailable :reason :missing_limit})) "unavailable"))
(each [_ fact (ipairs [{:type :missing} {:type :tokens :value math.huge}
                       {:type :percent :value -1} {:type :number :value (/ 0 0)}])]
  (assert (not (pcall misa.values.render fact))))
(local lines (misa.status.indicators db {:columns 200}))
(assert (= (. shared 1 :style) nil) "renderer mutated shared value spans")
(assert (= (. shared 1 :action) nil))
(local animated (accumulate [found nil _ span (ipairs (. lines 1 :spans)) &until found]
                  (when span.animation span)))
(assert animated "activity presentation lost its animation")
(assert (= animated.animation.id :animation/status))
(local presentation-model (misa.sub db [:indicators/model]))
(local swapped (misa.animations.swap db :test.still :status))
(assert (= (misa.sub swapped [:indicators/model]) presentation-model) "animation changed domain facts")
(assert (= (. (misa.animations.state swapped :status) :frames 1) "Z"))
(local still (misa.values.render {:type :activity :state :working}
                               {:activity_animation {:enabled false :frames ["a" "b"] :still "-"}}))
(assert (= (text still) "working-"))
(assert (= (. still 2 :animation) nil))
(assert (= (length (misa.values.render {:type :activity :state :ready}
                                     {:activity_animation (misa.animations.state db :status)})) 1))
(local button-model {:indicators [{:label "Model" :fact {:type :text :value "Example"}
                                   :action :models.open :hotkey :alt+m}]})
(local button (misa.components.render (misa.patch db {:hover_action :models.open})
                                     :status.indicators button-model {:columns 100}))
(local hover-background (. (misa.themes.style db :hover) :background))
(each [_ item (ipairs (. button.lines 1 :spans))]
  (assert (= item.action :models.open) "button separator lost its hit target")
  (assert (= (misa.json.encode item.style.background) (misa.json.encode hover-background))
          "button highlight must include its internal spaces"))
(output "indicator value contracts passed\n")
