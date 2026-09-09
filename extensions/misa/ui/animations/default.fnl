(local definitions (require :misa.definitions))

;; One terminal cell throughout: activity should not move surrounding text.
(fn []
  "Build the declarations for animation default."
  (definitions :animation.default
    [{:catalog :animations
      :id :default
      :value {:frames ["·" "•" "●" "•"] :still "…"}}
     {:catalog :animations :id :static :value {:frames ["…"]}}
     {:catalog :animations
      :id :spinner
      :value {:frames ["·" "•" "●" "•"]}}]
    {}))
