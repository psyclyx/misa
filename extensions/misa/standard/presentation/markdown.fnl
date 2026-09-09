(local markdown (require :misa.markdown))
(local render (require :misa.markdown.render))
{:services {:markdown {:parse markdown.parse
                       :new-document markdown.new-document}
            :markdown.view {:project render.project
                            :plain render.plain
                            :render render.render}}
 :requirements {:component.markdown [:layout :markdown :markdown.parse]}}
