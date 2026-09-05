;; One terminal cell throughout: activity should not move surrounding text.
{:setup (fn []
          {:fx [{:type :register/animation
                 :id :default
                 :value {:frames ["·" "•" "●" "•"] :still "…"}}
                {:type :register/animation
                 :id :static
                 :value {:frames ["…"]}}
                {:type :register/animation
                 :id :spinner
                 :value {:frames ["·" "•" "●" "•"]}}]})}
