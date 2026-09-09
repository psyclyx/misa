(local fennel (require :fennel))
(local setup (fennel.dofile :benchmarks/setup.fnl))
(local lua-dofile dofile)
(fn dofile [path]
  (if (path:match "%.fnl$") (fennel.dofile path) (lua-dofile path)))

(global misa {:json_null {}})

(setup (dofile :extensions/misa/json.fnl))

(local encode misa.json.encode)

(fn load [path]
  (global misa {})
  (setup (dofile path) {:config {}})
  misa.markdown.parse)

(local (baseline candidate) (values (load (. arg 1)) (load (. arg 2))))

(local cases ["plain `code` end"
              "***nested _emphasis_***"
              "[label](https://example.test)"
              "[unclosed"
              "``a ` b``"
              "\\*escaped\\*"
              "[a [b] c](target)"
              "~~~~lua\nhello\n~~~~"
              "|a|b|\n|---|---|\n|x|y|"])

(math.randomseed 15891)

(local alphabet "abc xyz`[]()*_~\\\n#|>-")

(for [_ 1 1000]
  (local parts {})
  (for [i 1 256]
    (local n (math.random (length alphabet)))
    (tset parts i (alphabet:sub n n)))
  (tset cases (+ (length cases) 1) (table.concat parts)))

(each [index source (ipairs cases)]
  (assert (= (encode (baseline source)) (encode (candidate source)))
          (.. "AST differs at case " index)))

(print (.. "byte-identical ASTs: " (length cases)))
