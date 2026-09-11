;; Stock composition contracts. Every module under `misa.standard` is reachable
;; from the stock application, and a directory's module names only modules under
;; its own directory. Together these mean a directory's list stays local and a
;; new file cannot be silently left out of the application.
(local read-file io.open)
(local output io.write)

(fn source [path]
  (with-open [file (assert (read-file path))]
    (file:read :*a)))

(fn module-name [path]
  (-> path
      (: :gsub :^extensions/ "")
      (: :gsub "/init%.fnl$" "")
      (: :gsub "%.fnl$" "")
      (: :gsub "/" ".")))

(fn requires [path]
  (icollect [name (: (source path) :gmatch "require :([%w%.-]+)")]
    name))

(fn directory [path] (path:match "^(.*)/[^/]+$"))

(fn under? [owner path]
  (= (path:sub 1 (+ (length owner) 1)) (.. owner "/")))

;; Modules deliberately outside the stock composition: provider fixtures and
;; presets that only tests or custom configuration select.
(local unstocked {:misa.standard.providers.command true
                  :misa.standard.providers.fake true
                  :misa.standard.providers.generic true})

(local modules {})
(each [path (: (source :src/standard_extensions/root.zig) :gmatch
               "%.path = \"([^\"]+%.fnl)\"")]
  (let [name (module-name (.. :extensions/ path))]
    (when (name:match "^misa%.standard")
      (tset modules name (.. :extensions/ path)))))

(local reachable {})
(fn visit [name]
  (when (and (. modules name) (not (. reachable name)))
    (tset reachable name true)
    (each [_ required (ipairs (requires (. modules name)))]
      (visit required))))

(visit :misa.standard)

(local missing (icollect [name (pairs modules)]
                 (when (and (not (. reachable name)) (not (. unstocked name)))
                   name)))

(table.sort missing)
(assert (= (length missing) 0)
        (.. "unreachable standard modules: " (table.concat missing ", ")))

(each [name path (pairs modules)]
  (when (path:match "/init%.fnl$")
    (let [owner (directory path)]
      (each [_ required (ipairs (requires path))]
        (when (required:match "^misa%.standard")
          (local target (. modules required))
          (assert target (.. path ": unknown standard module " required))
          (assert (under? owner target)
                  (.. path ": must name only modules under " owner)))))))

(output "extension composition contracts passed\n")
