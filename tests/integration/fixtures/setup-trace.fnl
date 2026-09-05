{:setup (fn [] (fn nested [] (error "setup exploded") nil) (nested) nil)}

