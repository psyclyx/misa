{:setup (fn []
          ;; One terminal cell throughout: activity should not move surrounding text.
          (misa.reg_animation :default
                              {:frames ["·" "•" "●" "•"] :still "…"})
          (misa.reg_animation :static {:frames ["…"]})
          (misa.reg_animation :spinner {:frames ["·" "•" "●" "•"]})
          nil)}

