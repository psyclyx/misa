(assert (and (and (and (= _G.os nil) (= _G.io nil)) (= _G.print nil)) (= _G.debug nil)))

(assert (and (and (= (type package) :table) (= package.loadlib nil))
             (= (type require) :function)))

(assert (and (and (and (= (type load) :function)
                       (= (type loadstring) :function))
                  (= (type loadfile) :function))
             (= (type dofile) :function)))

(assert (and (= _G.ffi nil) (= _G.jit nil)))

(local ok (pcall require :ffi))

(assert (not ok))

(assert (and (= (type misa.syntax) :table)
             (= (type misa.syntax.highlight) :function)))

(assert (= (length (misa.syntax.highlight :grammar_that_does_not_exist :plain))
           0))

(assert (not (pcall misa.syntax.highlight {} :plain)))

(assert (not (pcall misa.syntax.highlight :python {})))

(assert (not (pcall misa.syntax.highlight :python
                    (string.rep :x (+ (* 1024 1024) 1)))))

(tset package.preload :misa.test.module (fn [] {:answer 42}))

(assert (= (. (require :misa.test.module) :answer) 42))

{:setup (fn [context]
          (assert (= context.config.missing misa.json_null))
          (misa.reg_event :app/start
                          (fn []
                            {:fx [{:type :app/quit}]}))
          nil)}

