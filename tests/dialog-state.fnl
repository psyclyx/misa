(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local misa _G.misa)
(app.define ((fennel.dofile :extensions/json.fnl) {}))
(local specs ((fennel.dofile :extensions/dialogs.fnl)))
(local handlers {})
(each [_ spec (pairs (. specs :events))]
  (tset handlers spec.event spec.handler))
(app.define specs)
(app.define (definitions :test [{:catalog :dialog-inputs :id :clear :value (fn [] {:patch {:dialog {:input ""}}})}]))
(app.install)
(fn transition [db event]
  (local (before input) (values (misa.json.encode db) (misa.json.encode event)))
  (local result ((. handlers event.type) db event))
  (assert (= before (misa.json.encode db)) "dialog handler mutated state")
  (assert (= input (misa.json.encode event)) "dialog handler mutated event")
  (assert (not (and result result.db)))
  (values (misa.patch db (or (and result result.patch) {})) (and result result.fx)))
(local opening {:type :dialog/open :id :test :correlation :current :completion :test/finished
                :input true :cancellable true :hints [:hint]
                :actions [{:id :one} {:id :two :primary true}]})
(local initial (transition {} opening))
(local failure
       (G.for_all (G.vector (G.elements [{:type :dialog/input :kind :text :text "é"}
                                         {:type :dialog/input :kind :backspace}
                                         {:type :dialog/input :kind :arrow_left}
                                         {:type :dialog/input :kind :tab}
                                         {:type :dialog/input :kind :enter}
                                         {:type :dialog/input :kind :escape}
                                         {:type :dialog/update :id :test :correlation :current :actions [] :hints []}
                                         {:type :dialog/update :id :test :correlation :current :input false :cancellable false}
                                         {:type :dialog/close :id :test :correlation :stale}]))
                  (fn [events]
                    (var db initial)
                    (each [_ event (ipairs events)]
                      (when (not db.dialog) (set db (transition db opening)))
                      (local previous db)
                      (set db (transition db event))
                      (when db.dialog
                        (assert (= db.dialog.selected_action nil))
                        (assert (>= db.dialog.scroll 0)))
                      (when (= event.correlation :stale) (assert (= db previous)))))
                  {:cases 1000 :size 25}))
(assert (not failure) (and failure (fennel.view failure)))
(local cleared (transition initial {:type :dialog/update :id :test :correlation :current :actions [] :hints []}))
(assert (= (length cleared.dialog.actions) 0))
(assert (= (length cleared.dialog.hints) 0))
(local (finished fx) (transition initial {:type :dialog/input :kind :enter}))
(assert (= finished.dialog nil))
(assert (= (. fx 1 :event :action) :two))
(local protected (transition {} (misa.patch opening {:protected true :initial :not-retained})))
(assert (= protected.dialog.input ""))
(assert (= (transition protected {:type :dialog/input :kind :text :text :not-retained}) protected))
(local full (transition protected {:type :dialog/protected-input :id :test :correlation :current
                                   :length 65536 :too_long true :text :not-retained}))
(assert (= full.dialog.input_length 65536))
(assert full.dialog.input_error)
(assert (= full.dialog.input ""))
(local shorter (transition full {:type :dialog/protected-input :id :test :correlation :current :length 2}))
(assert (= shorter.dialog.input_error nil))
(local (cancelled cancel-fx) (transition shorter {:type :dialog/protected-input :id :test
                                                :correlation :current :cancelled true}))
(assert (= cancelled.dialog nil))
(assert (= (. cancel-fx 1 :event :protected) true))
(assert (= (. cancel-fx 1 :event :value) ""))
(assert (= (. cancel-fx 1 :event :cancelled) true))

(assert (= (transition protected {:type :dialog/input :kind :clear}) protected))
(output "dialog state properties passed\n")
