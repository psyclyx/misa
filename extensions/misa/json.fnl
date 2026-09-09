(local definitions (require :misa.definitions))

;; Provider-neutral JSON values used at Lua protocol boundaries.
(fn utf8 [codepoint]
  (if (<= codepoint 127) (string.char codepoint) (<= codepoint 2047)
      (string.char (+ 192 (math.floor (/ codepoint 64)))
                   (+ 128 (% codepoint 64))) (<= codepoint 65535)
      (string.char (+ 224 (math.floor (/ codepoint 4096)))
                   (+ 128 (% (math.floor (/ codepoint 64)) 64))
                   (+ 128 (% codepoint 64)))
      (string.char (+ 240 (math.floor (/ codepoint 262144)))
                   (+ 128 (% (math.floor (/ codepoint 4096)) 64))
                   (+ 128 (% (math.floor (/ codepoint 64)) 64))
                   (+ 128 (% codepoint 64)))))

(local simple-escapes {"\"" "\""
                       :/ "/"
                       "\\" "\\"
                       :b "\b"
                       :f "\f"
                       :n "\n"
                       :r "\r"
                       :t "\t"})

(fn decode [source]
  "Decode JSON into values, preserving null with misa.json-null."
  (assert (= (type source) :string) "JSON source must be a string")
  (var at 1)

  (fn fail [message]
    (error (.. message " at byte " (tostring at)) 0))

  (fn whitespace []
    (while (: (source:sub at at) :match "%s") (set at (+ at 1))))

  (var value nil)

  (fn unicode-escape []
    (let [hex (source:sub (+ at 1) (+ at 4))]
      (when (not (hex:match "^%x%x%x%x$")) (fail "invalid JSON unicode escape"))
      (var codepoint (tonumber hex 16))
      (set at (+ at 5))
      (if (<= 55296 codepoint 56319)
          (let [low (source:sub at (+ at 5))
                low-hex (low:match "^\\u(%x%x%x%x)$")
                low-code (and low-hex (tonumber low-hex 16))]
            (when (or (not low-code) (< low-code 56320) (> low-code 57343))
              (fail "invalid JSON surrogate pair"))
            (set codepoint (+ 65536 (* (- codepoint 55296) 1024)
                              (- low-code 56320)))
            (set at (+ at 6)))
          (<= 56320 codepoint 57343)
          (fail "invalid JSON surrogate"))
      (utf8 codepoint)))

  (fn string-value []
    (when (not= (source:sub at at) "\"") (fail "expected JSON string"))
    (set at (+ at 1))
    (let [parts []]
      (var start at)
      (var closed false)
      (while (and (<= at (length source)) (not closed))
        (let [byte (source:byte at)]
          (if (= byte 34)
              (do
                (table.insert parts (source:sub start (- at 1)))
                (set at (+ at 1))
                (set closed true))
              (< byte 32)
              (fail "unescaped control character in JSON string")
              (= byte 92)
              (do
                (table.insert parts (source:sub start (- at 1)))
                (set at (+ at 1))
                (let [escape (source:sub at at)
                      simple (. simple-escapes escape)]
                  (if simple
                      (do
                        (table.insert parts simple)
                        (set at (+ at 1)))
                      (= escape :u)
                      (table.insert parts (unicode-escape))
                      (fail "invalid JSON escape")))
                (set start at))
              (set at (+ at 1)))))
      (if closed (table.concat parts) (fail "unterminated JSON string"))))

  (fn array-value []
    (set at (+ at 1))
    (whitespace)
    (let [result []]
      (var closed (= (source:sub at at) "]"))
      (when closed (set at (+ at 1)))
      (while (not closed)
        (table.insert result (value))
        (whitespace)
        (let [delimiter (source:sub at at)]
          (set at (+ at 1))
          (if (= delimiter "]") (set closed true)
              (not= delimiter ",") (fail "expected ',' or ']' in JSON array")
              (whitespace))))
      result))

  (fn object-value []
    (set at (+ at 1))
    (whitespace)
    (let [result {}]
      (var closed (= (source:sub at at) "}"))
      (when closed (set at (+ at 1)))
      (while (not closed)
        (let [key (string-value)]
          (whitespace)
          (when (not= (source:sub at at) ":")
            (fail "expected ':' in JSON object"))
          (set at (+ at 1))
          (whitespace)
          (tset result key (value))
          (whitespace)
          (let [delimiter (source:sub at at)]
            (set at (+ at 1))
            (if (= delimiter "}") (set closed true)
                (not= delimiter ",") (fail "expected ',' or '}' in JSON object")
                (whitespace)))))
      result))

  (fn number-value []
    (let [token (: (source:sub at) :match "^([^,%]%}%s]+)")
          integer (and token
                       (or (token:match :^-?0$) (token:match "^-?[1-9]%d*$")))
          decimal (and token
                       (or (token:match "^-?0%.%d+$")
                           (token:match "^-?[1-9]%d*%.%d+$")))
          exponent (and token
                        (or (token:match "^-?0[eE][+-]?%d+$")
                            (token:match "^-?[1-9]%d*[eE][+-]?%d+$")
                            (token:match "^-?0%.%d+[eE][+-]?%d+$")
                            (token:match "^-?[1-9]%d*%.%d+[eE][+-]?%d+$")))]
      (if (or integer decimal exponent)
          (do
            (set at (+ at (length token)))
            (tonumber token))
          (fail "invalid JSON value"))))

  (fn literal-value [literal decoded]
    (set at (+ at (length literal)))
    decoded)

  (set value
       (fn []
         (whitespace)
         (let [first (source:sub at at)]
           (if (= first "\"") (string-value)
               (= first "[") (array-value)
               (= first "{") (object-value)
               (= (source:sub at (+ at 3)) :true) (literal-value :true true)
               (= (source:sub at (+ at 4)) :false) (literal-value :false false)
               (= (source:sub at (+ at 3)) :null) (literal-value :null
                                                                 misa.json-null)
               (number-value)))))
  (let [result (value)]
    (whitespace)
    (when (<= at (length source)) (fail "trailing JSON data"))
    result))

(local escapes {"\b" "\\b"
                "\t" "\\t"
                "\n" "\\n"
                "\f" "\\f"
                "\r" "\\r"
                "\"" "\\\""
                "\\" "\\\\"})

(fn encode [root]
  "Encode a value as JSON, rejecting cycles and non-finite numbers."
  (let [active {}]
    (var visit nil)

    (fn table-value [value]
      (assert (not (. active value)) "cyclic JSON value")
      (tset active value true)
      (var count 0)
      (var maximum 0)
      (var array true)
      (each [key (pairs value)]
        (set count (+ count 1))
        (if (or (not= (type key) :number) (< key 1) (not= (% key 1) 0))
            (set array false)
            (set maximum (math.max maximum key))))
      (set array (and array (> count 0) (= maximum count)))
      (let [parts []]
        (if array
            (for [index 1 count]
              (tset parts index (visit (. value index))))
            (let [keys []]
              (each [key (pairs value)]
                (assert (= (type key) :string)
                        "JSON object keys must be strings")
                (table.insert keys key))
              (table.sort keys)
              (each [_ key (ipairs keys)]
                (table.insert parts (.. (visit key) ":" (visit (. value key)))))))
        (tset active value nil)
        (.. (if array "[" "{") (table.concat parts ",") (if array "]" "}"))))

    (set visit (fn [value]
                 (let [kind (type value)]
                   (if (= value misa.json-null)
                       :null
                       (= kind :string)
                       (.. "\""
                           (value:gsub "[%z\001-\031\\\"]"
                                       (fn [char]
                                         (or (. escapes char)
                                             (string.format "\\u%04x"
                                                            (char:byte)))))
                           "\"")
                       (= kind :boolean)
                       (if value :true :false)
                       (= kind :number)
                       (do
                         (assert (and (= value value) (not= value math.huge)
                                      (not= value (- math.huge)))
                                 "JSON numbers must be finite")
                         (tostring value))
                       (do
                         (assert (= kind :table)
                                 (.. "unsupported JSON value: " kind))
                         (table-value value))))))
    (visit root)))

(fn build []
  "Build the declarations for json."
  (let [declarations []]
    (table.insert declarations
                  {:catalog :services :id :json :value {: decode : encode}})
    (definitions.build :json declarations {})))

{: build}
