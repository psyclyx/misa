(local fennel (require :fennel))
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local context {:argv [] :config {:choices {:preview_renderers {:custom :test.preview}}}})
(each [_ name (ipairs [:json :keybindings :layout :values :choices :choices/preview :choices/layout])]
  (app.define ((fennel.dofile (.. :extensions/ name :.fnl)) context)))
(local shared [{:spans [{:text "CUSTOM" :action :custom.action :link "https://example.test"
                        :animation {:id :preview :interval_ms 100 :frames [{:text "CUSTOM"} {:text "custom"}]}}]}])
(var received nil)
(var calls 0)
(app.define (definitions :fixture [{:catalog :choice-previews :id :test.preview :value (fn [model render-context]
                                     (set received model)
                                     (set calls (+ calls 1))
                                     (assert (> render-context.columns 0)) shared)}]))
(app.define {:choice-previews {:test.override (fn [model] shared)}})
(app.install)
(fn texts [lines]
  (table.concat (icollect [_ line (ipairs lines)]
                 (table.concat (icollect [_ span (ipairs line.spans)] span.text))) "\n"))
(local preview {:type :model :title :priced/model :context_window 100000
                :cost {:currency :USD :token_unit 1000000 :estimated true
                       :pricing {:input 2 :output 8 :cache_read 0 :request 0.01}}})
(local before (misa.json.encode preview))
(local wide (texts (misa.choices.preview preview {:columns 120})))
(assert (wide:find "Context: 100000 tokens" 1 true))
(assert (wide:find "$2 input · $8 output" 1 true))
(assert (wide:find "$0 read" 1 true) "zero cache price was hidden")
(assert (wide:find "Per request: $0.0100" 1 true))
(assert (= (texts (misa.choices.preview preview {:columns 120 :compact true})) "$2 in / $8 out per 1M"))
(assert (= before (misa.json.encode preview)))
(assert (texts (misa.choices.preview {:type :model :context_window 10
                                   :cost {:unavailable true}} {:columns 80})))
(assert (not (pcall misa.choices.preview {:type :unknown} {:columns 80})))
(local fact {:type :custom :pricing {:input 0.1234567} :context_window 12345})
(local session (misa.choices.session {:title :Models :items [{:id :custom :value :custom :preview fact}]} {}))
(local geometry (misa.choices.picker-layout session {} {:columns 80 :available_lines 20}))
(assert (= calls 1) "preview geometry rendered more than once")
(assert (= (misa.json.encode received) (misa.json.encode fact)) "preview renderer did not receive raw facts")
(assert (= geometry.preview.model received))
(assert (= (. geometry.preview.lines 1 :spans 1 :action) :custom.action))
(assert (= (. geometry.preview.lines 1 :spans 1 :link) "https://example.test"))
(assert (= (. shared 1 :spans 1 :text) "CUSTOM"))
(local picker ((fennel.dofile :extensions/component/picker.fnl) {}))
(local rendered ((. picker :components :default.picker :render) geometry))
(assert (= calls 1) "picker repeated preview geometry")
(local rendered-preview (. rendered.lines (+ geometry.preview.y 1)))
(local custom-span (assert (accumulate [found nil _ span (ipairs rendered-preview.spans) &until found]
                            (when (= span.action :custom.action) span))))
(assert (= custom-span.link "https://example.test"))
(assert (= custom-span.animation.id :preview) "picker discarded preview animation metadata")
(each [_ width (ipairs [1 5 20 80])]
  (local compact (misa.choices.completion-layout session {} width 4))
  (assert (<= (+ compact.panel_y compact.panel_height compact.hint_height) compact.height))
  (each [_ line (ipairs compact.preview.lines)]
    (assert (<= (misa.layout.width (texts [line])) compact.width))))
;; Existing preview types can select an extension renderer through configuration.
(local override ((fennel.dofile :extensions/choices/preview.fnl)
                 {:config {:choices {:preview_renderers {:model :test.override}}}}))

(local project (. override.services :choices.preview))


(assert (= (texts (project preview {:columns 80})) "CUSTOM"))
(output "choice preview contracts passed\n")
