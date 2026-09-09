(local fennel (require :fennel))
(local output io.write)
(local G (require :tests.generators))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local app ((require :tests.application) {:argv [] :config {}}))
(local definitions (require :misa.definitions))
(local misa _G.misa)
(local specs ((fennel.dofile :extensions/misa/editor/images/init.fnl) {:config {}}))
(local handlers {})
(each [_ spec (pairs (. specs :events))]
  (tset handlers spec.event spec.handler))
(local actions [{:type :images/paste}
                {:type :images/load :path :test.png}
                {:type :images/load :path ""}
                {:type :images/loaded :id "image:1" :ok false}
                {:type :images/loaded :id "image:2" :ok true
                 :data {:width 1 :height 1 :mime_type :image/png :data :encoded}}
                {:type :agent/reset}])
(local failure
       (G.for_all (G.vector (G.elements actions))
                  (fn [events]
                    (var db {})
                    (var sequence 0)
                    (local pending {})
                    (each [_ event (ipairs events)]
                      (local before (fennel.view db))
                      (local result ((. handlers event.type) db event))
                      (assert (= before (fennel.view db)) "image handler mutated input")
                      (local fx (or (and result result.fx) []))
                      (if (or (= event.type :images/paste) (= event.type :images/load))
                          (do
                            (set sequence (+ sequence 1))
                            (local id (.. "image:" sequence))
                            (if (= event.path "")
                                (assert (= (. fx 1 :event :level) :error))
                                (do
                                  (tset pending id true)
                                  (assert (= (. fx 1 :id) id)))))
                          (= event.type :images/loaded)
                          (if (. pending event.id)
                              (do
                                (tset pending event.id nil)
                                (assert (= (. fx 1 :event :type)
                                           (if event.ok :editor/attach :transcript/harness))))
                              (assert (= result nil) "accepted stale completion"))
                          (= event.type :agent/reset)
                          (do
                            (each [_ effect (ipairs fx)]
                              (assert (= effect.type :operation/cancel))
                              (assert (. pending effect.id) "cancelled unknown operation")
                              (tset pending effect.id nil))
                            (assert (= (next pending) nil) "missed pending cancellation")))
                      (when result
                        (assert (not result.db))
                        (set db (misa.patch db result.patch)))
                      (assert (= (or (and db.images db.images.next_id) 0) sequence))
                      (assert (= (fennel.view (or (and db.images db.images.pending) {}))
                                 (fennel.view pending)) "pending requests diverged")))
                  {:cases 1000 :size 30}))
(assert (not failure) (and failure (fennel.view failure)))
(output "image state properties passed\n")
