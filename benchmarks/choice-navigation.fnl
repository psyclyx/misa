;; Compare this service path against saved choices.fnl and choice_layout.fnl.
;; MISA_CHOICE_BASELINE selects that directory; output contains deterministic
;; frame oracles plus sample CPU milliseconds per navigation-and-layout step.
(local fennel (require :fennel))
(local output io.write)
(local clock os.clock)
(local baseline (os.getenv :MISA_CHOICE_BASELINE))
(fennel.dofile :src/lua_runtime/framework.fnl)
(local misa _G.misa)
(local app ((require :tests.application) {:config {} :argv []}))
(each [_ name (ipairs [:misa.json
                       :misa.ui.layout
                       :misa.keybindings
                       :misa.choices
                       :misa.ui.values
                       :misa.choices.preview :misa.models.preview
                       :misa.choices.layout])]
  (local saved (and baseline (. {:misa.choices :choices
                                 :misa.choices.layout :choice_layout}
                                name)))
  (local module (if saved (fennel.dofile (.. baseline "/" saved ".fnl"))
                    (require name)))
  (app.include module {:config {} :argv []}))

(app.install)
(each [_ count (ipairs [100 1000 3000])]
  (local items
         (fcollect [index 1 count]
           {:id (.. :provider/model- index)
            :label (.. :provider/model- index)
            :description "A sample model with enough detail to wrap"}))
  (local initial (misa.choices.session {:title :Models
                                        :items items
                                        :views [:browse]}
                                       {}))
  (local terminal {:columns 100 :lines 32})

  (fn run []
    (var session initial)
    (var frame nil)
    (for [_ 1 12]
      (set session (. (misa.choices.input session {:action :next} {}) :session))
      (set frame (misa.choices.picker-layout session {} terminal)))
    frame)

  (local oracle (misa.json.encode (run)))
  (for [_ 1 6]
    (assert (= oracle (misa.json.encode (run)))
            "non-deterministic navigation projection"))
  (output (.. count " oracle " oracle "\n"))
  (for [_ 1 10]
    (local start (clock))
    (local frame (run))
    (local elapsed (- (clock) start))
    (assert (= oracle (misa.json.encode frame))
            "timed navigation changed output")
    (output (string.format "%d sample %.6f\n" count (* 1000 (/ elapsed 12))))))
