;; Shared orderless fuzzy matching for every choice surface.
(fn words [value]
  (let [result []]
    (each [word (: (value:lower) :gmatch "%S+")]
      (table.insert result word))
    result))

(fn subsequence [needle haystack]
  (var at 1)
  (var index 1)
  (var first nil)
  (var previous nil)
  (var gaps 0)
  (var result nil)
  (while (and (<= index (length haystack)) (not result))
    (when (= (haystack:sub index index) (needle:sub at at))
      (set first (or first index))
      (when previous (set gaps (+ gaps (- index previous 1))))
      (set previous index)
      (set at (+ at 1))
      (when (> at (length needle)) (set result (+ first (* gaps 2)))))
    (set index (+ index 1)))
  result)

(fn score [query text]
  (let [lower (text:lower)]
    (var total 0)
    (each [_ word (ipairs (words query))]
      (when total
        (let [exact (lower:find word 1 true)
              part (if exact (- exact 1) (subsequence word lower))]
          (set total (when part (+ total part))))))
    total))

(fn rank-before [left right]
  (let [lv (or left.item.value left.item.name "")
        rv (or right.item.value right.item.name "")]
    (if (not= left.score right.score) (< left.score right.score)
        (not= lv rv) (< lv rv)
        (< left.ordinal right.ordinal))))

{:setup (fn []
  (set misa.fuzzy_score score)
  (fn misa.fuzzy_choices [source query text]
    (let [ranked []
          result []]
      (each [ordinal item (ipairs source)]
        (let [extra (if (= (type item.search) :string) item.search
                        (= (type item.search) :table) (table.concat item.search " ")
                        "")
              searchable (or (and text (text item))
                             (.. item.value " " (or item.label "") " "
                                 (or item.description "") " " extra))
              rank (if (= query "") 0 (score query searchable))]
          (when rank (table.insert ranked {: item : ordinal :score rank}))))
      (table.sort ranked rank-before)
      (each [_ value (ipairs ranked)] (table.insert result value.item))
      result)))}
