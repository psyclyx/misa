package org.misa.app

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

enum class Phase { Idle, Connecting, Pairing, Connected, Closed, Failed }
data class Notice(val level: String, val text: String)
data class DownloadFile(val path: String, val name: String)
data class PendingAttachment(val name: String, val blob: JSONObject)

data class UiState(
    val phase: Phase = Phase.Idle,
    val message: String = "",
    val session: Session? = null,
    val view: Node? = null,
    val notices: List<Notice> = emptyList(),
    val status: String = "",
    val draft: String = "",
    val ticket: String = "",
    val images: Map<String, String> = emptyMap(),
    val imageErrors: Map<String, String> = emptyMap(),
    val attachments: List<PendingAttachment> = emptyList(),
    val uploading: Boolean = false,
    val download: DownloadFile? = null,
) { val connected: Boolean get() = phase == Phase.Connected }

/** The platform owns its files and preferences; the native client owns protocol synchronization. */
class MisaViewModel(application: Application) : AndroidViewModel(application) {
    private val memory = application.getSharedPreferences("client", 0)
    private val _state = MutableStateFlow(UiState(draft = memory.getString("draft", "") ?: "", ticket = memory.getString("ticket", "") ?: ""))
    val state: StateFlow<UiState> = _state.asStateFlow()
    private val handle = AtomicLong(0)
    private var generation = 0L
    private val requestedImages = mutableSetOf<String>()
    private var saving = false
    private val open = MutableStateFlow(memory.getStringSet("expanded", emptySet())!!.toSet())
    val expanded: StateFlow<Set<String>> = open.asStateFlow()

    fun toggle(node: String) {
        open.update { if (node in it) it - node else it + node }
        memory.edit().putStringSet("expanded", open.value).apply()
    }
    fun draft(text: String) {
        _state.update { it.copy(draft = text) }
        memory.edit().putString("draft", text).apply()
    }
    fun resume() {
        if (memory.getBoolean("resume", false) && state.value.ticket.isNotBlank()) connect(state.value.ticket)
    }
    fun pause() {
        saving = false
        generation++
        val current = handle.getAndSet(0)
        if (current != 0L) Native.disconnect(current)
        _state.update { it.copy(phase = Phase.Closed, uploading = false, message = "offline — showing saved conversation") }
    }
    fun connect(ticket: String) {
        val trimmed = ticket.trim()
        if (trimmed.isEmpty()) return
        if (handle.get() != 0L) {
            if (trimmed == state.value.ticket) return
            pause()
        }
        val sameSession = trimmed == state.value.ticket
        val attempt = ++generation
        requestedImages.clear()
        _state.update { it.copy(phase = Phase.Connecting, ticket = trimmed, view = if (sameSession) it.view else null, attachments = if (sameSession) it.attachments else emptyList(), imageErrors = emptyMap()) }
        memory.edit().putString("ticket", trimmed).putBoolean("resume", true).apply()
        val listener = Listener { json ->
            viewModelScope.launch(Dispatchers.Main.immediate) {
                if (attempt == generation) onEvent(json)
            }
        }
        val created = Native.connect(trimmed, File(getApplication<Application>().filesDir, "session-client").absolutePath, listener)
        handle.set(created)
        if (created == 0L) _state.update { it.copy(phase = Phase.Failed, message = "the native client could not start") }
    }
    fun disconnect() {
        memory.edit().putBoolean("resume", false).apply()
        pause()
    }
    fun prompt(text: String) {
        if (text.isBlank() && state.value.attachments.isEmpty()) return
        if (send(Intents.prompt(text, state.value.attachments.map { it.blob }))) {
            _state.update { it.copy(attachments = emptyList()) }
            draft("")
        }
    }
    fun removeAttachment(index: Int) { _state.update { it.copy(attachments = it.attachments.filterIndexed { at, _ -> at != index }) } }
    fun command(name: String, values: List<Pair<String, String>> = emptyList()) { send(Intents.command(name, values)) }
    fun action(node: String, action: Action, fields: List<FieldValue> = emptyList()) {
        if (action.id == "attachment.save") {
            if (saving) return
            saving = true
        }
        if (!send(Intents.action(node, action, fields))) saving = false
    }
    fun cancel() { send(Intents.cancel()) }
    private fun send(intent: String): Boolean {
        val current = handle.get()
        if (current != 0L && Native.send(current, intent)) return true
        announce("error", "the connection is not open")
        return false
    }
    fun image(hash: String) {
        if (!hash.matches(Regex("[0-9a-f]{64}")) || hash in state.value.images || !requestedImages.add(hash)) return
        viewModelScope.launch {
            val cached = withContext(Dispatchers.IO) { Native.cached(File(getApplication<Application>().filesDir, "session-client").absolutePath, hash) }
            if (cached != null) _state.update { it.copy(images = it.images + (hash to cached)) }
            else if (!state.value.connected || !send(JSONObject().put("local", "fetch").put("hash", hash).toString())) requestedImages.remove(hash)
        }
    }
    fun attach(uri: Uri) {
        _state.update { it.copy(uploading = true) }
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val app = getApplication<Application>()
                    val resolver = app.contentResolver
                    var name = "attachment"
                    resolver.query(uri, arrayOf(android.provider.OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                        if (cursor.moveToFirst()) name = cursor.getString(0) ?: name
                    }
                    val file = File.createTempFile("upload-", ".tmp", app.cacheDir)
                    try {
                        resolver.openInputStream(uri)?.use { input ->
                            file.outputStream().use { output ->
                                val buffer = ByteArray(65536)
                                var total = 0L
                                while (true) {
                                    val count = input.read(buffer)
                                    if (count < 0) break
                                    total += count
                                    require(total <= 32L * 1024 * 1024) { "attachments are limited to 32 MiB" }
                                    output.write(buffer, 0, count)
                                }
                            }
                        } ?: error("could not open the selected file")
                        JSONObject().put("local", "upload").put("path", file.absolutePath).put("name", name)
                            .put("media", resolver.getType(uri) ?: "application/octet-stream").toString()
                    } catch (error: Exception) { file.delete(); throw error }
                }
            }
            result.onSuccess {
                if (!send(it)) { File(JSONObject(it).getString("path")).delete(); _state.update { state -> state.copy(uploading = false) } }
            }
                .onFailure { announce("error", it.message ?: "upload failed"); _state.update { state -> state.copy(uploading = false) } }
        }
    }
    fun saveDownload(uri: Uri?) {
        val download = state.value.download ?: return
        _state.update { it.copy(download = null) }
        saving = false
        if (uri == null) return
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    DocumentFiles.save(getApplication<Application>().contentResolver, File(download.path), uri)
                }
            }.onSuccess { announce("info", "saved ${download.name}") }
                .onFailure { announce("error", it.message ?: "save failed") }
        }
    }
    private fun announce(level: String, text: String) { _state.update { it.copy(notices = (it.notices + Notice(level, text)).takeLast(200)) } }
    private fun onEvent(json: String) {
        val event = runCatching { JSONObject(json) }.getOrNull() ?: return
        when (event.optString("kind")) {
            "state" -> {
                val phase = when (event.optString("state")) { "connecting" -> Phase.Connecting; "pairing" -> Phase.Pairing; "connected" -> Phase.Connected; "closed" -> Phase.Closed; else -> state.value.phase }
                _state.update { it.copy(phase = phase, message = event.optString("message")) }
            }
            "ticket" -> { val ticket = event.getString("ticket"); memory.edit().putString("ticket", ticket).apply(); _state.update { it.copy(ticket = ticket) } }
            "paired" -> announce("info", event.optString("message", "paired"))
            "session" -> event.optJSONObject("session")?.let { session -> _state.update { it.copy(session = Wire.parseSession(session)) } }
            "view" -> event.optJSONObject("view")?.let { view -> _state.update { it.copy(view = Wire.parseNode(view)) } }
            "blob" -> _state.update { it.copy(images = it.images + (event.getString("hash") to event.getString("path"))) }
            "blob_failed" -> { saving = false; _state.update { it.copy(imageErrors = it.imageErrors + (event.getString("hash") to event.optString("text"))) }; announce("error", event.optString("text")) }
            "uploaded" -> _state.update { it.copy(uploading = false, attachments = it.attachments + PendingAttachment(event.getString("name"), event.getJSONObject("blob"))) }
            "upload_failed" -> { _state.update { it.copy(uploading = false) }; announce("error", event.optString("text")) }
            "download" -> _state.update { it.copy(download = DownloadFile(event.getString("path"), File(event.optString("name", "attachment")).name.filter { it.code >= 32 }.take(160).ifBlank { "attachment" })) }
            "notice" -> { if (event.optString("level") == "error") saving = false; announce(event.optString("level", "info"), event.optString("text")) }
            "status" -> _state.update { it.copy(status = event.optString("text")) }
            "recover" -> draft(event.optString("text"))
            "fault" -> {
                saving = false
                if (state.value.phase == Phase.Connecting || state.value.phase == Phase.Pairing) {
                    handle.set(0); _state.update { it.copy(phase = Phase.Failed, message = event.optString("message")) }
                } else announce("error", event.optString("message"))
            }
            "closed" -> { handle.set(0); _state.update { it.copy(phase = Phase.Closed, message = "connection closed — saved conversation is available") } }
        }
    }
    override fun onCleared() { pause(); super.onCleared() }
}
