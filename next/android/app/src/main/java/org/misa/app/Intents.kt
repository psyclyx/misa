package org.misa.app

import org.json.JSONArray
import org.json.JSONObject

/**
 * The four things a client may ask for, as JSON.
 *
 * This is the whole of the client's vocabulary: submit text, resolve an
 * affordance the session offered, invoke a command the session declared, or
 * cancel. Nothing here can name an effect, a model parameter, or a state path,
 * which is what makes the phone exactly as powerful as the terminal and no more
 * — and no less.
 */
object Intents {
    fun prompt(text: String, attachments: List<JSONObject> = emptyList()): String = JSONObject().put("intent", "prompt").put("text", text).put("attachments", JSONArray(attachments)).toString()

    fun cancel(): String = JSONObject().put("intent", "cancel").toString()

    /**
     * A command and its arguments.
     *
     * One argument is sent as a bare string, because that is what a person typed
     * and what the session's own positional rule accepts; two or more become a
     * named object. A client that always sent a map would work and would make the
     * session invent a name for a single positional value.
     */
    fun command(name: String, values: List<Pair<String, String>>): String {
        val body = JSONObject().put("intent", "command").put("name", name)
        val given = values.filter { it.second.isNotEmpty() }
        when {
            given.isEmpty() -> Unit
            given.size == 1 -> body.put("args", given[0].second)
            else -> {
                val args = JSONObject()
                given.forEach { (key, value) -> args.put(key, value) }
                body.put("args", args)
            }
        }
        return body.toString()
    }

    /** Resolve an action a node offered, with the field values it carried. */
    fun action(node: String, action: Action, fields: List<FieldValue> = emptyList()): String {
        val body = JSONObject().put("intent", "action").put("action", action.id)
        if (node.isNotEmpty()) body.put("node", node)
        if (action.args != null && action.args.length() > 0) body.put("args", action.args)
        if (fields.isNotEmpty()) {
            val list = JSONArray()
            fields.forEach { field ->
                list.put(
                    JSONObject()
                        .put("id", field.id)
                        .put("label", field.label)
                        .put("value", field.value)
                        .put("kind", JSONObject().put("shape", "inline")),
                )
            }
            body.put("fields", list)
        }
        return body.toString()
    }
}

/** One value a form's field carried back. */
data class FieldValue(val id: String, val label: String, val value: String)

/** The fact formatter, mirroring `misa-render`'s rules for the roles it knows. */
object Facts {
    fun format(role: String, value: Any?): String =
        when (role) {
            "value.money" -> money(asLong(value))
            "value.tokens" -> count(asLong(value), "tok")
            "value.count" -> count(asLong(value), "")
            "value.percent" -> percent(value)
            "value.ratio" -> (asDouble(value)?.let { percent(it * 100.0) } ?: plain(value))
            "value.duration" -> duration(asLong(value))
            "value.bytes" -> bytes(asLong(value))
            else -> plain(value)
        }

    private fun money(micros: Long?): String {
        val micros = micros ?: 0L
        val sign = if (micros < 0) "-" else ""
        val magnitude = kotlin.math.abs(micros)
        val whole = magnitude / 1_000_000
        val cents = (magnitude % 1_000_000) / 10_000
        if (whole == 0L && cents == 0L && magnitude > 0) return "$sign<\$0.01"
        return "$sign\$$whole.${cents.toString().padStart(2, '0')}"
    }

    private fun count(number: Long?, unit: String): String {
        val number = number ?: return ""
        val sign = if (number < 0) "-" else ""
        val magnitude = kotlin.math.abs(number)
        val written =
            when {
                magnitude <= 9_999 -> magnitude.toString()
                magnitude <= 999_999 -> String.format("%.1fk", magnitude / 1_000.0)
                magnitude <= 999_999_999 -> String.format("%.1fM", magnitude / 1_000_000.0)
                else -> String.format("%.1fB", magnitude / 1_000_000_000.0)
            }
        return if (unit.isEmpty()) "$sign$written" else "$sign$written $unit"
    }

    private fun percent(value: Any?): String {
        val number = asDouble(value) ?: return plain(value)
        return if (number == kotlin.math.floor(number)) String.format("%.0f%%", number)
        else String.format("%.1f%%", number)
    }

    private fun duration(millis: Long?): String {
        val millis = (millis ?: return "").coerceAtLeast(0)
        return when {
            millis < 1_000 -> "${millis}ms"
            millis < 60_000 -> String.format("%.1fs", millis / 1_000.0)
            millis < 3_600_000 -> "${millis / 60_000}m${((millis % 60_000) / 1_000).toString().padStart(2, '0')}s"
            else -> "${millis / 3_600_000}h${((millis % 3_600_000) / 60_000).toString().padStart(2, '0')}m"
        }
    }

    private fun bytes(bytes: Long?): String {
        val bytes = bytes ?: return ""
        val sign = if (bytes < 0) "-" else ""
        val magnitude = kotlin.math.abs(bytes)
        val (scaled, unit) =
            when {
                magnitude < 1_000 -> magnitude.toDouble() to "B"
                magnitude < 1_000_000 -> magnitude / 1_000.0 to "kB"
                magnitude < 1_000_000_000 -> magnitude / 1_000_000.0 to "MB"
                else -> magnitude / 1_000_000_000.0 to "GB"
            }
        return if (unit == "B") "$sign${String.format("%.0f", scaled)} $unit"
        else "$sign${String.format("%.1f", scaled)} $unit"
    }

    private fun plain(value: Any?): String =
        when (value) {
            null, JSONObject.NULL -> ""
            is String -> value
            is Boolean -> if (value) "yes" else "no"
            is Double -> if (value == kotlin.math.floor(value)) value.toLong().toString() else value.toString()
            else -> value.toString()
        }

    private fun asLong(value: Any?): Long? =
        when (value) {
            is Int -> value.toLong()
            is Long -> value
            is Double -> value.toLong()
            is String -> value.toLongOrNull()
            else -> null
        }

    private fun asDouble(value: Any?): Double? =
        when (value) {
            is Int -> value.toDouble()
            is Long -> value.toDouble()
            is Double -> value
            is String -> value.toDoubleOrNull()
            else -> null
        }
}
