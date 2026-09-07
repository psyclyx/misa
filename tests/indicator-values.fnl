(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local context {:argv [] :config {:themes {:persist false} :components {:persist false}
                                  :animations {:persist false}
                                  :status {:indicators [:zero :false :hidden :custom :activity]}}})
(each [_ name (ipairs [:json :layout :themes :theme/default :components :values :component/status
                       :animations :animation/default :indicators])]
  (misa._setup (fennel.dofile (.. :extensions/ name :.fnl)) context))
(local calls {})
(each [_ id (ipairs [:zero :false :hidden :custom :activity])]
  ;; Deliberately register the consumer before its query.
  (misa._setup_effects {:fx [{:type :register/indicator :value {: id :query [(.. :test/ id)]}}
                            {:type :register/sub
                             :value {:id (.. :test/ id) :inputs [[:db/path :facts id]]
                                     :compute (fn [inputs]
                                                (tset calls id (+ (or (. calls id) 0) 1))
                                                (. inputs 1))}}]}))
(local shared [{:text "extension"}])
(misa._setup_effects {:fx [{:type :register/animation :id :test.still :value {:frames ["Z"]}}
                          {:type :register/value-renderer :id :custom
                           :render (fn [fact] (assert (= fact.value 7)) shared)}]})
(each [_ malformed (ipairs [{:id :legacy :value (fn [] :text)}
                            {:id :empty :query []}
                            {:id :invalid :query [false]}
                            {:id :zero :query [:test/zero]}])]
  (assert (not (pcall misa._setup_effects {:fx [{:type :register/indicator :value malformed}]}))))
(assert (not (pcall misa._setup_effects {:fx [{:type :register/value-renderer :id :text :render (fn [])}]})))
(var observed nil)
(misa._setup_effects {:fx [{:type :register/event :name :test/read
                           :handler (fn [db] (set observed db) nil)}]})
(misa._seal context)
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
(assert (= (text (misa.render_value {:type :tokens :value 1234})) "1.2k"))
(assert (= (text (misa.render_value {:type :ratio :used 1234 :unit :tokens})) "1.2k/?"))
(assert (= (text (misa.render_value {:type :percent :value 0 :basis :remaining})) "0% left"))
(assert (= (text (misa.render_value {:type :boolean :value false})) "false"))
(assert (= (text (misa.render_value {:type :money :amount 0 :currency :USD})) "$0"))
(assert (= (text (misa.render_value {:type :unavailable :reason :missing_limit})) "unavailable"))
(each [_ fact (ipairs [{:type :missing} {:type :tokens :value math.huge}
                       {:type :percent :value -1} {:type :number :value (/ 0 0)}])]
  (assert (not (pcall misa.render_value fact))))
(local lines (misa.indicators_projection db {:columns 200}))
(assert (= (. shared 1 :style) nil) "renderer mutated shared value spans")
(assert (= (. shared 1 :action) nil))
(local animated (accumulate [found nil _ span (ipairs (. lines 1 :spans)) &until found]
                  (when span.animation span)))
(assert animated "activity presentation lost its animation")
(assert (= animated.animation.id :animation/status))
(local presentation-model (misa.sub db [:indicators/model]))
(local swapped (misa.swap_animation db :test.still :status))
(assert (= (misa.sub swapped [:indicators/model]) presentation-model) "animation changed domain facts")
(assert (= (. (misa.animation_presentation swapped :status) :frames 1) "Z"))
(local still (misa.render_value {:type :activity :state :working}
                               {:activity_animation {:enabled false :frames ["a" "b"] :still "-"}}))
(assert (= (text still) "working-"))
(assert (= (. still 2 :animation) nil))
(assert (= (length (misa.render_value {:type :activity :state :ready}
                                     {:activity_animation (misa.animation_presentation db :status)})) 1))
(output "indicator value contracts passed\n")
