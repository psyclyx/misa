(local definitions (require :misa.definitions))

;; Dialog chrome composes content, inputs, and shared buttons.
(fn span [text style link] {: text :style (or style :dialog.message) : link})
(fn append [target source]
  (each [_ value (ipairs source)] (table.insert target value)))

(fn render [model context]
  (local width (math.max 1 (- (or context.columns 80) 2)))
  (local room (math.max 0 (or context.available_lines 24)))
  (local body [])

  (fn add [spans]
    (each [_ line (ipairs (misa.layout.wrap-spans [{: spans}] width))]
      (table.insert line.spans 1 (span "│ " :dialog.label))
      (table.insert body line)))

  (each [_ field (ipairs [{:key :message :style :dialog.message}
                          {:key :url :label "URL  " :style :link :link true}
                          {:key :code :label "Code " :style :dialog.code}
                          {:key :progress :style :dialog.progress}
                          {:key :input_error :style :dialog.message}])]
    (local value (. model field.key))
    (when (and value (not= value ""))
      (add [(span (.. (or field.label "") (tostring value)) field.style
                  (when field.link value))])))
  (local content (if (and model.content model.content.role)
                     (. (context.render_child model.content.role
                                              model.content.model
                                              {:columns width})
                        :lines)
                     (or model.content [])))
  (each [_ line (ipairs content)] (add line.spans))
  (local buttons (icollect [_ action (ipairs (or model.actions []))]
                   (when (not action.inline) action)))
  (local footer-spans (misa.components.buttons buttons model))
  (each [_ hint (ipairs (or model.hints []))]
    (table.insert footer-spans (span (.. "   " hint) :dialog.hint)))

  (fn wrap-footer []
    (local wrapped
           (misa.layout.wrap-spans [{:spans footer-spans}]
                                   (math.max 1 (- width 1))))
    (each [index line (ipairs wrapped)]
      (table.insert line.spans 1
                    (span (if (= index 1) "└─ " "   ") :dialog.label)))
    wrapped)

  (var footer (wrap-footer))
  (var input nil)
  (when model.input_enabled
    (local value (if model.protected
                     (string.rep "•"
                                 (math.min (or model.input_length 0)
                                           (math.max 1 (- width 2))))
                     (or model.input "")))
    (set input (misa.layout.wrap-input value (or context.columns 80)
                                       (length value) "│ " :dialog.input
                                       :dialog.label)))
  (var input-room (if input
                      (math.min (length input.lines)
                                (math.max 1 (- room (length footer) 1)))
                      0))
  (var body-room (math.max 0 (- room (length footer) input-room 1)))
  (when (> (length body) body-room)
    (table.insert footer-spans (span "   "))
    (append footer-spans
            (misa.keybindings.reference [{:key :arrow_up :label ""}
                                         {:key :arrow_down :label "Scroll"}]))
    (set footer (wrap-footer))
    (set input-room (if input
                        (math.min (length input.lines)
                                  (math.max 1 (- room (length footer) 1)))
                        0))
    (set body-room (math.max 0 (- room (length footer) input-room 1))))
  (local maximum (math.max 0 (- (length body) body-room)))
  (local offset (math.min maximum (math.max 0 (or model.scroll 0))))
  (local lines
         [{:spans [(span "┌─ " :dialog.label)
                   (span (or model.title "Interaction") :dialog.title)]}])
  (for [index (+ offset 1) (math.min (length body) (+ offset body-room))]
    (table.insert lines (. body index)))
  (var cursor nil)
  (when (and input (> room 1))
    (local first (math.max 1 (+ (- input.cursor.row input-room) 1)))
    (local before (length lines))
    (for [index first (math.min (length input.lines) (+ first input-room -1))]
      (table.insert lines (. input.lines index)))
    (set cursor {:byte input.cursor.byte
                 :row (+ before (- input.cursor.row first) 1)}))
  (append lines footer)
  (while (> (length lines) room)
    (table.remove lines 1)
    (when cursor (set cursor.row (- cursor.row 1))))
  (when (and cursor (or (< cursor.row 1) (> cursor.row (length lines))))
    (set cursor nil))
  {: lines
   : cursor
   :max_scroll maximum
   :scroll_page (math.max 1 body-room)
   :overlay true
   :surface :surface.dialog})

(fn []
  "Build the declarations for component dialog."
  (definitions :component.dialog
    [{:catalog :components :id :default.dialog :value {:compose true : render}}]
    {:requirements {:component.dialog [:layout :components.buttons]}}))
