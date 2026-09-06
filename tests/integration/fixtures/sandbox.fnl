(assert (and (and (and (= _G.os nil) (= _G.io nil)) (= _G.print nil))
             (= _G.debug nil)))

(assert (and (and (= (type package) :table) (= package.loadlib nil))
             (= (type require) :function)))

(assert (and (and (and (= (type load) :function)
                       (= (type loadstring) :function))
                  (= (type loadfile) :function))
             (= (type dofile) :function)))

(assert (and (= _G.ffi nil) (= _G.jit nil)))

(local ok (pcall require :ffi))

(assert (not ok))

(assert (= misa.syntax nil) "synchronous syntax capability remains exposed")

(tset package.preload :misa.test.module (fn [] {:answer 42}))

(assert (= (. (require :misa.test.module) :answer) 42))

{:setup (fn [context]
          (local setup-fx [])
          (assert (= context.config.missing misa.json_null))
          (table.insert setup-fx
                        {:type :register/event
                         :name :app/start
                         :handler (fn []
                                    {:fx [{:type :app/quit}]})})
          nil
          {:fx setup-fx})}
