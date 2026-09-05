{:setup (fn []
          (misa.reg_event :app/start (fn [] (error :exploded) nil))
          nil)}

