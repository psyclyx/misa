(fn integer? [value minimum maximum]
  "Return whether a value is an integer within inclusive bounds."
  (and (= (type value) :number) (= value (math.floor value)) (>= value minimum)
       (<= value maximum)))

(fn array [value label]
  (assert (= (type value) :table) (.. label " must be an array"))
  (each [key _ (pairs value)]
    (assert (integer? key 1 (length value)) (.. label " must be a dense array")))
  value)

(fn printable [text]
  (assert (and (= (type text) :string) (not (text:find "[%z\1-\31\127]")))
          "span text must be printable text on one line"))

(fn validate [rendered]
  "Validate a semantic component view and return the original value."
  (assert (= (type rendered) :table) "component render must return a table")
  (each [_ line (ipairs (array rendered.lines :lines))]
    (assert (= (type line) :table) "line must be a table")
    (each [_ span (ipairs (array line.spans :spans))]
      (assert (= (type span) :table) "span must be a table")
      (printable span.text)
      (each [_ field (ipairs [:action :link])]
        (when (. span field)
          (assert (= (type (. span field)) :string) (.. field " must be text"))))
      (when span.animation
        (assert (= (type span.animation) :table) "animation must be a table")
        (each [_ frame (ipairs (array span.animation.frames :frames))]
          (assert (= (type frame) :table) "animation frame must be a table")
          (each [key _ (pairs frame)]
            (assert (or (= key :text) (= key :style))
                    "invalid animation frame field"))
          (when frame.text (printable frame.text)))
        (assert (and (= (type span.animation.id) :string)
                     (not= span.animation.id "")
                     (integer? span.animation.interval_ms 10 60000)
                     (integer? (length span.animation.frames) 1 64)
                     (integer? (or span.animation.phase 0) 0
                               (- (length span.animation.frames) 1)))
                "invalid animation timing or identity")))
    (when line.image
      (let [image line.image]
        (assert (= (type image) :table) "image must be a table")
        (each [_ pair (ipairs [[:id 4294967295]
                               [:width 480]
                               [:height 320]
                               [:columns 65535]
                               [:rows 65535]])]
          (assert (integer? (. image (. pair 1)) 1 (. pair 2))
                  "invalid image geometry"))
        (when image.column
          (assert (integer? image.column 1 65535) "invalid image column"))
        (assert (and (= image.format :rgba) (= (type image.data) :string))
                "invalid image payload"))))
  (when rendered.cursor
    (let [cursor rendered.cursor]
      (assert (and (= (type cursor) :table)
                   (integer? cursor.row 1 (length rendered.lines)))
              "invalid cursor row")
      (let [spans (. rendered.lines cursor.row :spans)]
        (assert (and (= cursor.column nil)
                     (or (= cursor.shape nil) (= cursor.shape :bar)
                         (= cursor.shape :block)))
                "invalid cursor shape or column")
        (var bytes 0)
        (each [_ span (ipairs spans)]
          (set bytes (+ bytes (length span.text))))
        (assert (integer? cursor.byte 0 bytes) "invalid cursor byte")
        (var start 0)
        (each [_ span (ipairs spans)]
          (when (and (>= cursor.byte start)
                     (< cursor.byte (+ start (length span.text))))
            (let [byte (span.text:byte (+ (- cursor.byte start) 1))]
              (assert (or (< byte 128) (>= byte 192))
                      "cursor byte splits a UTF-8 character")))
          (set start (+ start (length span.text)))))))
  rendered)

{:view validate :integer? integer?}
