package org.misa.app

import androidx.lifecycle.ViewModel
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject

enum class Phase {
    Idle,
    Connecting,
    Pairing,
    Connected,
    Closed,
    Failed,
}

data class Notice(val level: String, val text: String)

/**
 * Everything the phone is showing, as one value.
 *
 * One state object rather than a dozen fields because the native client delivers
 * events on its own thread and a `StateFlow` is the thing that makes "the last
 * event wins, in order" true without a lock in the UI.
 */
data class UiState(
    val phase: Phase = Phase.Idle,
    val message: String = "",
    val session: Session? = null,
    val view: Node? = null,
    val notices: List<Notice> = emptyList(),
    val status: String = "",
) {
    val connected: Boolean get() = phase == Phase.Connected
}

/**
 * The client, as Compose sees it.
 *
 * The state it holds is *presentation* state — a view tree, a notice list, a
 * status line — and the decisions it makes are local ones: which node to expand,
 * what to type, whether to show the palette. Everything else is an intent.
 */
class MisaViewModel : ViewModel() {
    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    private val handle = AtomicLong(0)

    /** The node ids a person has opened, so an expansion survives a re-render. */
    private val open = MutableStateFlow<Set<String>>(emptySet())
    val expanded: StateFlow<Set<String>> = open.asStateFlow()

    fun toggle(node: String) {
        open.update { if (node in it) it - node else it + node }
    }

    fun connect(ticket: String) {
        if (handle.get() != 0L) return
        val trimmed = ticket.trim()
        if (trimmed.isEmpty()) {
            _state.update { it.copy(phase = Phase.Failed, message = "paste a ticket or scan the daemon's code") }
            return
        }
        _state.update { UiState(phase = Phase.Connecting) }
        val listener = Listener(::onEvent)
        val created = Native.connect(trimmed, listener)
        if (created == 0L) {
            _state.update { it.copy(phase = Phase.Failed, message = "the native client could not start") }
        } else {
            handle.set(created)
        }
    }

    fun disconnect() {
        val current = handle.getAndSet(0)
        if (current != 0L) Native.disconnect(current)
        _state.update { it.copy(phase = Phase.Closed, message = "detached") }
    }

    /** Submit a turn. */
    fun prompt(text: String) {
        if (text.isBlank()) return
        send(Intents.prompt(text))
    }

    fun command(name: String, values: List<Pair<String, String>> = emptyList()) {
        send(Intents.command(name, values))
    }

    fun action(node: String, action: Action, fields: List<FieldValue> = emptyList()) {
        send(Intents.action(node, action, fields))
    }

    fun cancel() {
        send(Intents.cancel())
    }

    private fun send(intent: String) {
        val current = handle.get()
        if (current == 0L || !Native.send(current, intent)) {
            announce("error", "the connection is not open")
        }
    }

    private fun announce(level: String, text: String) {
        _state.update { it.copy(notices = (it.notices + Notice(level, text)).takeLast(200)) }
    }

    /** One event from the native client, already JSON. */
    private fun onEvent(json: String) {
        val event = runCatching { JSONObject(json) }.getOrNull() ?: return
        when (event.optString("kind")) {
            "state" -> onState(event)
            "paired" -> announce("info", event.optString("message", "paired"))
            "session" -> event.optJSONObject("session")?.let { session ->
                _state.update { it.copy(session = Wire.parseSession(session), phase = Phase.Connected) }
            }
            "view" -> event.optJSONObject("view")?.let { view ->
                _state.update { it.copy(view = Wire.parseNode(view)) }
            }
            "notice" -> announce(event.optString("level", "info"), event.optString("text", ""))
            "status" -> _state.update { it.copy(status = event.optString("text", "")) }
            "delta" -> Unit // A delta is a convenience; the next view carries the same text in full.
            "fault" -> onFault(event)
            "closed" -> _state.update { it.copy(phase = Phase.Closed, message = "the connection closed") }
        }
    }

    private fun onState(event: JSONObject) {
        val phase =
            when (event.optString("state")) {
                "connecting" -> Phase.Connecting
                "pairing" -> Phase.Pairing
                "connected" -> Phase.Connected
                "closed" -> Phase.Closed
                else -> null
            }
        val message = event.optString("message", "")
        _state.update { state ->
            if (phase != null) state.copy(phase = phase, message = message) else state.copy(message = message)
        }
    }

    /**
     * A fault during setup is the end of the attempt; a fault during a session is
     * a line, because a refused intent must not detach a working client.
     */
    private fun onFault(event: JSONObject) {
        val message = event.optString("message", "something went wrong")
        val code = event.optString("code", "")
        val settingUp = _state.value.phase == Phase.Connecting || _state.value.phase == Phase.Pairing
        if (settingUp) {
            _state.update { it.copy(phase = Phase.Failed, message = message) }
        } else {
            announce("error", if (code.isEmpty()) message else "$code: $message")
        }
    }

    override fun onCleared() {
        disconnect()
        super.onCleared()
    }
}
