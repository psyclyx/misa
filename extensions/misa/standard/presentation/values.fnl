(local formatting (require :misa.ui.values))
{:services {:values.timestamp->seconds formatting.timestamp-seconds
            :values.render formatting.values-render}
 :value-renderers formatting.builtins
 :validators {:value-renderers formatting.validate-renderer}}
