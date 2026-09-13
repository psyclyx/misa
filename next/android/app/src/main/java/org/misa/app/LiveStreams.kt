package org.misa.app

import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.staticCompositionLocalOf
import org.json.JSONArray
import org.json.JSONObject

data class LiveText(val id: String, val role: String, val text: String, val bytes: Int)
class LiveStreams {
    val values = mutableStateMapOf<String, LiveText>()
    fun reset(streams: JSONArray = JSONArray()) {
        values.clear()
        for (i in 0 until streams.length()) current(streams.getJSONObject(i))
    }
    private fun current(stream: JSONObject) {
        val id = stream.getString("id")
        val text = stream.getString("text")
        values[id] = LiveText(id, stream.getString("role"), text, text.toByteArray(Charsets.UTF_8).size)
    }
    fun apply(update: JSONObject) {
        val id = update.optString("id")
        when (update.getString("update")) {
            "current" -> current(update.getJSONObject("stream"))
            "append" -> {
                val previous = values.getValue(id)
                check(previous.bytes == update.getInt("offset"))
                val suffix = update.getString("text")
                values[id] = previous.copy(text = previous.text + suffix, bytes = previous.bytes + suffix.toByteArray(Charsets.UTF_8).size)
            }
            "end" -> values.remove(id)
        }
    }
}
internal data class StreamDisplay(val streams: LiveStreams, val contains: (String) -> Boolean)
internal val LocalStreams = staticCompositionLocalOf { StreamDisplay(LiveStreams()) { false } }
