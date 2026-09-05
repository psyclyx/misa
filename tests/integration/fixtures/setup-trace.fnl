{:setup (fn []
          (local setup-fx [])

          (fn nested [] (error "setup exploded") nil)

          (nested)
          nil
          {:fx setup-fx})}
