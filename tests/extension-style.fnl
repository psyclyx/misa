(local fennel (require :fennel))
(local read-file io.open)
(local output io.write)
(fennel.dofile :src/lua_runtime/framework.fnl)

(fn source [path]
  (with-open [file (assert (read-file path))]
    (file:read "*a")))

(fn function? [form]
  (and (fennel.list? form) (= (tostring (. form 1)) :fn)))

(fn documented? [form]
  (= (type (. form (if (fennel.sym? (. form 2)) 4 3))) :string))

(fn audit [path]
  (let [forms (icollect [_ form (fennel.parser (fennel.stringStream (source path)))]
                form)
        functions {}]
    (fn visit [form top-level?]
      (when (= (type form) :table)
        (when (fennel.list? form)
          (let [head (tostring (. form 1))]
            (assert (not (and (= head :local) (not top-level?)))
                    (.. path ": use let inside expressions"))
            (assert (not= head :lua) (.. path ": use Fennel control flow"))
            (when (and top-level? (function? form) (fennel.sym? (. form 2)))
              (let [name (tostring (. form 2))]
                (assert (not (. functions name))
                        (.. path ": duplicate function " name))
                (tset functions name form)))))
        (each [key value (pairs form)]
          (when (= (type key) :table) (visit key false))
          (visit value false))))

    (each [_ form (ipairs forms)] (visit form true))
    (let [exports (. forms (length forms))]
      (let [name (-> path (: :gsub "^extensions/" "")
                     (: :gsub "/init%.fnl$" "") (: :gsub "%.fnl$" "")
                     (: :gsub "/" "."))
            data (require name)]
        (assert (= (type data) :table) (.. path ": return module data"))
        (assert (= data.build nil)
                (.. path ": declaration builders are not module APIs")))
      (each [name value (pairs (if (and (= (type exports) :table)
                                        (not (fennel.list? exports))
                                        (not (fennel.sym? exports)))
                                   exports
                                   {}))]
        (let [implementation (if (function? value) value
                                 (and (fennel.sym? value)
                                      (. functions (tostring value))))]
          (when implementation
            (assert (documented? implementation)
                    (.. path ": public function " (tostring name)
                        " needs a docstring"))))))))

(each [path (: (source :src/standard_extensions/root.zig) :gmatch
               "%.path = \"([^\"]+%.fnl)\"")]
  (audit (.. "extensions/" path)))

(output "extension style contracts passed\n")
