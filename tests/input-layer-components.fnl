;; Input-docked controls use the same semantic component/hover path as other UI.
(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local context {:argv [] :config {:themes {:persist false} :components {:persist false}}})
(local layers {})
(each [_ name (ipairs [:misa.json :misa.ui.themes :misa.ui.themes.default :misa.ui.components :misa.ui.layout :misa.editor.queue.view :misa.editor.attachments])]
  (local specs ((. (require name) :build) context))
  (each [id spec (pairs (or specs.view-layers {}))]
    (tset layers id spec.handler))
  (app.define specs))
(var observed nil)
(var received nil)
(app.define (definitions.build :fixture [{:catalog :events  :value {:event :test/read :handler (fn [db] (set observed db) nil)}}
       {:catalog :components :id :default.attachment.fixture :value {:render (fn [] {:lines []})}}
       {:catalog :components :id :custom.controls :value {:render (fn [model] (set received model)
                         {:lines [{:spans [{:text :custom :style :plain}]}]})}}]))
(app.install)
(each [_ event (ipairs [{:type :app/start} {:type :test/read}])]
  (misa._dispatch event {:columns 80 :lines 24 :interactive true} {:wall_ms 0 :monotonic_ms 0})
  (misa._commit))
(local db (misa.patch observed {:queue {:pending "first\nsecond" :attachments [{:type :fixture}]}
                               :editor {:attachments [{:type :fixture}]}
                               :images {:pending {:loading true}}}))
(local cofx {:terminal {:columns 80 :images false}})
(local before (misa.json.encode db))
(fn action-span [view id]
  (each [_ line (ipairs view.lines)]
    (each [_ span (ipairs line.spans)]
      (when (= span.action id) (lua "return span"))))
  (error (.. "missing action: " id)))
(each [_ spec (ipairs [{:layer :pending-prompt :action :queue.edit}
                       {:layer :pending-prompt :action :queue.steer}
                       {:layer :draft-attachments :action :images.remove}])]
  (local layer (. layers spec.layer))
  (local rest (layer db cofx))
  (local encoded (misa.json.encode rest))
  (local normal (action-span rest spec.action))
  (assert (= (misa.json.encode normal.style) (misa.json.encode (misa.themes.style db :keybinding)))
          "resting appearance changed")
  (local hovered (layer (misa.patch db {:hover_action spec.action}) cofx))
  (local highlighted (action-span hovered spec.action))
  (assert (= (misa.json.encode highlighted.style.background)
             (misa.json.encode (. (misa.themes.style db :hover) :background))))
  (assert (not= highlighted.style.background normal.style.background))
  (assert (= (misa.json.encode highlighted.style.foreground)
             (misa.json.encode normal.style.foreground)))
  (assert (= highlighted.text normal.text))
  (assert (= encoded (misa.json.encode rest)) "hover mutated reusable output")
  (assert (= encoded (misa.json.encode (layer db cofx))) "hover did not clear"))
(assert (= before (misa.json.encode db)))
(local queued ((. layers :pending-prompt) db cofx))
(assert (= (. queued.lines 2 :spans 1 :text) "first ↵ second  [1 image(s)]"))
(local attachments ((. layers :draft-attachments) db cofx))
(assert (= (. attachments.lines 1 :spans 1 :text) "Loading image…"))
(local custom-queue ((. layers :pending-prompt)
                    (misa.components.swap db :pending-prompt :custom.controls) cofx))
(assert (= received.pending "first\nsecond") "view layer pre-rendered queue text")
(assert (= received.attachment_count 1))
(assert (= (. custom-queue.lines 1 :spans 1 :text) :custom))
((. layers :draft-attachments) (misa.components.swap db :attachment-controls :custom.controls) cofx)
(assert (= received.count 1))
(assert (= received.pending true))
(assert (= ((. layers :pending-prompt) observed cofx) nil))
(assert (= ((. layers :draft-attachments) observed cofx) nil))
(output "input layer component contracts passed\n")
