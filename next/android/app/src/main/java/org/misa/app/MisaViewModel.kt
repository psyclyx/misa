package org.misa.app

import android.app.Application
import android.net.Uri
import androidx.compose.runtime.snapshots.Snapshot
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.yield
import org.json.JSONArray
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

data class DownloadFile(val path: String, val name: String)

data class PendingAttachment(val name: String, val blob: JSONObject)

data class DaemonRow(
    val id: String,
    val freshness: String,
    val sessions: List<SessionRow>,
    val work: List<WorkRow> = emptyList(),
)

data class WorkRow(
    val id: String,
    val scope: JSONObject,
    val state: String,
    val blocking: Boolean,
    val available: Boolean,
)

data class RequestTarget(
    val daemon: String,
    val scope: JSONObject,
    val id: String,
    val generation: Long,
    val title: String,
    val available: Boolean,
)

data class SessionRow(
    val title: String,
    val scope: JSONObject,
    val summary: String = "",
    val requests: List<RequestTarget> = emptyList(),
)

data class InstanceRow(
    val id: String,
    val daemon: String,
    val title: String,
    val available: Boolean,
)

data class PresentationChoice(
    val id: String,
    val title: String,
    val selected: String,
    val variants: List<String>,
)

data class RequestAction(val id: String, val label: String)

data class FormField(
    val id: String,
    val optional: Boolean,
    val text: Boolean,
    val label: String = id,
)

data class LocalForm(
    val action: String,
    val fields: List<FormField>,
    val direct: Boolean = false,
    val daemon: String? = null,
    val scope: JSONObject? = null,
    val drafts: Map<String, String> = emptyMap(),
)

data class InputRequest(
    val id: String,
    val generation: Long,
    val title: String,
    val body: Node,
    val input: String?,
    val label: String,
    val secret: Boolean,
    val actions: List<RequestAction>,
    val fields: List<FormField> = emptyList(),
)

data class UiState(
    val phase: Phase = Phase.Idle,
    val message: String = "",
    val instance: String = "",
    val daemon: String = "",
    val session: Session? = null,
    val view: Node? = null,
    val panels: Map<String, Node> = emptyMap(),
    val notices: List<Notice> = emptyList(),
    val status: String = "",
    val draft: String = "",
    val ticket: String = "",
    val images: Map<String, String> = emptyMap(),
    val imageErrors: Map<String, String> = emptyMap(),
    val attachments: List<PendingAttachment> = emptyList(),
    val uploading: Boolean = false,
    val download: DownloadFile? = null,
    val daemons: List<DaemonRow> = emptyList(),
    val instances: List<InstanceRow> = emptyList(),
    val presentations: List<PresentationChoice> = emptyList(),
    val requests: List<InputRequest> = emptyList(),
    val report: Node? = null,
    val completions: List<Pair<String, String>> = emptyList(),
    val form: LocalForm? = null,
    val archives: Map<String, List<Pair<String, String>>> = emptyMap(),
    val daemonCommands: Map<String, List<String>> = emptyMap(),
    val theme: String = "system",
    val hiddenStatus: Set<String> = emptySet(),
    val requestFocus: String? = null,
    val renderProblem: String? = null,
) {
    val connected: Boolean
        get() = phase == Phase.Connected
}

private class Document {
    val tree = ViewTree()
    val streams = LiveStreams()
}

private class Page(val key: String, val daemon: String, val scope: JSONObject, var state: UiState) {
    val requestDrafts = mutableMapOf<String, String>()
    val documents = mutableMapOf<String, Document>()
    val requested = mutableSetOf<String>()
    var composition = 0L
    var render = 0L
    var requiredRender = 0L
    var recovering = false
    var recoveryAttempts = 0
    var saving = false
    var completionRequest = ""
    var open = emptySet<String>()
}

/** One workspace owns relationships; local pages own drafts and presentation state. */
class MisaViewModel
@JvmOverloads
constructor(
    application: Application,
    private val decodeEvent: (String) -> JSONObject = { JSONObject(it) },
) : AndroidViewModel(application) {
    private val memory = application.getSharedPreferences("client", 0)
    private var theme = memory.getString("theme", "system") ?: "system"
    private var hiddenStatus =
        memory.getStringSet("hiddenStatus", emptySet())?.toSet() ?: emptySet()
    private var pendingRequest: RequestTarget? = null
    private val _state =
        MutableStateFlow(
            UiState(
                ticket = memory.getString("ticket", "") ?: "",
                theme = theme,
                hiddenStatus = hiddenStatus,
            )
        )
    val state: StateFlow<UiState> = _state.asStateFlow()
    private var idleState = _state.value
    private val open = MutableStateFlow<Set<String>>(emptySet())
    val expanded: StateFlow<Set<String>> = open.asStateFlow()
    private val pages = linkedMapOf<String, Page>()
    private val nativePages = mutableMapOf<String, Page>()
    private var selected: Page? = null
    private var restoreSelection: Pair<String, JSONObject>? = null
    private var restoreCached = true
    private var handle = 0L
    private var generation = 0L
    private var draining = false
    private var downloadOwner: Pair<Page, DownloadFile>? = null
    private var daemons = emptyList<DaemonRow>()
    private var instances = emptyList<InstanceRow>()
    private val archives = mutableMapOf<String, List<Pair<String, String>>>()
    private val archiveRequests = mutableMapOf<String, String>()
    private val emptyStreams = LiveStreams()
    val streams: LiveStreams
        get() = selected?.documents?.get("conversation")?.streams ?: emptyStreams

    fun containsNode(id: String): Boolean =
        selected?.documents?.get("conversation")?.tree?.contains(id) == true

    fun panelStreams(id: String): LiveStreams =
        selected?.documents?.get(id)?.streams ?: emptyStreams

    fun panelContains(slot: String, id: String): Boolean =
        selected?.documents?.get(slot)?.tree?.contains(id) == true

    private fun storage() = File(getApplication<Application>().filesDir, "session-client")

    private fun publish(page: Page? = selected) {
        if (page !== selected) return
        pendingRequest?.let { target ->
            if (
                page != null &&
                    page.key == pageKey(target.daemon, target.scope) &&
                    page.state.requests.any {
                        it.id == target.id && it.generation == target.generation
                    }
            ) {
                page.state = page.state.copy(requestFocus = target.id)
                pendingRequest = null
            }
        }
        val base = page?.state ?: idleState
        val retained =
            pages.values
                .filter { it !in nativePages.values }
                .map {
                    InstanceRow(
                        "local:${it.key}",
                        it.daemon,
                        it.state.session?.title ?: "Saved session",
                        false,
                    )
                }
        _state.value =
            base.copy(
                daemons = daemons,
                instances = instances + retained,
                ticket = state.value.ticket,
                archives = archives.toMap(),
                daemonCommands = daemonCommands.toMap(),
                theme = theme,
                hiddenStatus = hiddenStatus,
            )
        open.value = page?.open ?: emptySet()
    }

    private fun update(page: Page? = selected, transform: (UiState) -> UiState) {
        if (page == null) {
            idleState = transform(idleState)
            publish()
        } else {
            page.state = transform(page.state)
            publish(page)
        }
    }

    private fun announce(text: String, level: String = "error", page: Page? = selected) =
        update(page) { it.copy(notices = (it.notices + Notice(level, text)).takeLast(100)) }

    fun toggle(node: String) {
        selected?.let { page ->
            page.open = if (node in page.open) page.open - node else page.open + node
            publish(page)
        }
    }

    fun draft(text: String) {
        selected?.let { page ->
            update(page) { it.copy(draft = text) }
            memory.edit().putString("draft.${page.key}", text).apply()
        }
    }

    fun resume() {
        if (handle == 0L) start("")
    }

    private fun start(target: String) {
        restoreCached = target.isEmpty()
        restoreSelection = if (restoreCached) selected?.let { it.daemon to it.scope } else null
        val attempt = ++generation
        val listener = Listener { json ->
            viewModelScope.launch(Dispatchers.Main.immediate) {
                if (attempt != generation) return@launch
                val event = runCatching { JSONObject(json) }.getOrNull() ?: return@launch
                if (event.optString("kind") != "wake") {
                    announce(event.optString("message"))
                    return@launch
                }
                if (draining) return@launch
                draining = true
                try {
                    var count = 0
                    while (attempt == generation) {
                        val next = Native.poll(handle) ?: break
                        var event: JSONObject? = null
                        try {
                            event = decodeEvent(next)
                            onEvent(event)
                        } catch (error: Exception) {
                            val page = event?.optString("instance")?.let { nativePages[it] }
                            if (page != null && event.optString("kind") == "transaction") {
                                renderFailed(page, error.message.orEmpty())
                            } else if (event == null) {
                                nativePages.values.distinct().forEach {
                                    renderFailed(it, error.message.orEmpty())
                                }
                            } else announce("Presentation error: ${error.message}")
                        }
                        if (++count % 8 == 0) yield()
                    }
                } finally {
                    draining = false
                }
            }
        }
        handle = Native.connect(target, storage().absolutePath, listener)
        if (handle == 0L) {
            announce("Native workspace could not start")
            return
        }
        val targets = JSONObject(memory.getString("relationshipTargets", "{}") ?: "{}")
        val saved =
            if (targets.length() > 0)
                targets.keys().asSequence().map { targets.getString(it) }.toSet()
            else memory.getStringSet("relationships", emptySet()) ?: emptySet()
        saved
            .filter { it != target }
            .take(32)
            .forEach {
                sendLocal(
                    JSONObject().put("local", "connect").put("target", it).put("select", false)
                )
            }
    }

    fun pause() {
        generation++
        draining = false
        val current = handle
        handle = 0L
        if (current != 0L) Native.disconnect(current)
        nativePages.clear()
        instances = emptyList()
        Snapshot.withMutableSnapshot {
            pages.values.forEach { page ->
                page.documents.values.forEach { it.streams.reset() }
                page.state =
                    page.state.copy(
                        phase = Phase.Closed,
                        uploading = false,
                        message = "Offline · retained canonical content",
                    )
                page.saving = false
            }
            publish()
        }
    }

    fun connect(target: String) {
        if (target.isBlank()) return
        restoreSelection = null
        restoreCached = false
        _state.value = _state.value.copy(ticket = target.trim())
        if (handle == 0L) start(target.trim())
        else sendLocal(JSONObject().put("local", "connect").put("target", target.trim()))
    }

    fun disconnect() {
        selected?.let { disconnectDaemon(it.daemon) }
    }

    fun disconnectDaemon(id: String) {
        sendLocal(JSONObject().put("local", "disconnect").put("daemon", id))
    }

    private val daemonCommands = mutableMapOf<String, List<String>>()

    fun daemonCommands(id: String) {
        sendLocal(JSONObject().put("local", "daemon_commands").put("daemon", id))
    }

    fun daemonForm(daemon: String, command: String) {
        sendLocal(
            JSONObject().put("local", "daemon_form").put("daemon", daemon).put("command", command)
        )
    }

    fun workForm(daemon: String, work: WorkRow, command: String) {
        if (!work.available) {
            announce("Work owner unavailable")
            return
        }
        sendLocal(
            JSONObject()
                .put("local", "daemon_form")
                .put("daemon", daemon)
                .put("command", command)
                .put("scope", work.scope)
                .put("operation", work.id)
        )
    }

    fun createSession(daemon: String, id: String, conversation: String) {
        val input = JSONObject().put("id", id)
        if (conversation.isNotBlank()) input.put("conversation", conversation)
        lifecycle(
            daemon,
            if (conversation.isBlank()) "daemon.session.create" else "daemon.session.resume",
            input,
        )
    }

    fun conversations(daemon: String, prefix: String) {
        archiveRequests[daemon] = prefix
        sendLocal(JSONObject().put("local", "archives").put("daemon", daemon).put("prefix", prefix))
    }

    fun closeSession(daemon: String, scope: JSONObject) {
        lifecycle(
            daemon,
            "daemon.session.close",
            JSONObject()
                .put("id", scope.getJSONObject("id").getString("id"))
                .put("incarnation", scope.getString("incarnation")),
        )
    }

    private fun lifecycle(daemon: String, command: String, input: JSONObject) {
        if (
            !sendLocal(
                JSONObject()
                    .put("local", "lifecycle")
                    .put("daemon", daemon)
                    .put("command", command)
                    .put("input", input)
            )
        )
            announce("Daemon command queue unavailable")
    }

    fun select(daemon: String, scope: JSONObject) {
        pendingRequest?.let {
            if (pageKey(it.daemon, it.scope) != pageKey(daemon, scope)) pendingRequest = null
        }
        if (pages.size >= 8 && !pages.containsKey(pageKey(daemon, scope))) {
            pendingRequest = null
            announce("Close a local session before opening another")
            return
        }
        sendLocal(JSONObject().put("local", "select").put("daemon", daemon).put("scope", scope))
    }

    fun theme(value: String) {
        if (value !in listOf("system", "dark", "light")) return
        theme = value
        memory.edit().putString("theme", value).apply()
        publish()
    }

    fun statusField(id: String, visible: Boolean) {
        val stored = memory.getStringSet("hiddenStatus", emptySet()).orEmpty().toSet()
        hiddenStatus = if (visible) stored - id else stored + id
        memory.edit().putStringSet("hiddenStatus", hiddenStatus).apply()
        publish()
    }

    fun navigateRequest(target: RequestTarget) {
        if (
            !target.available ||
                daemons.none { daemon ->
                    daemon.id == target.daemon &&
                        daemon.sessions.any {
                            pageKey(daemon.id, it.scope) == pageKey(target.daemon, target.scope)
                        }
                }
        ) {
            announce("Request source is stale; reconnect before opening it")
            return
        }
        pendingRequest = target
        select(target.daemon, target.scope)
    }

    fun consumeRequestFocus() {
        update { it.copy(requestFocus = null) }
    }

    private fun refresh(page: Page) {
        page.requiredRender = page.render + 1
        page.recovering = true
        page.documents.clear()
        if (
            !send(
                JSONObject()
                    .put("local", "refresh")
                    .put("composition", page.composition)
                    .toString(),
                page,
            )
        ) {
            update(page) {
                it.copy(
                    renderProblem = "Presentation refresh could not be queued; reconnect or retry"
                )
            }
        }
    }

    private fun renderFailed(page: Page, reason: String) {
        page.recoveryAttempts++
        page.documents.clear()
        update(page) {
            it.copy(renderProblem = "Presentation cache failed; retained content is stale")
        }
        announce("Presentation cache failed: $reason", page = page)
        if (page.recoveryAttempts == 1) refresh(page)
    }

    fun retryRendering() {
        selected?.let { page ->
            page.recoveryAttempts = 0
            refresh(page)
        }
    }

    fun selectInstance(id: String) {
        pendingRequest = null
        if (id.startsWith("local:")) {
            selected = pages[id.removePrefix("local:")]
            publish()
        } else sendLocal(JSONObject().put("local", "select").put("instance", id))
    }

    fun closeInstance(id: String) {
        if (id.startsWith("local:")) {
            pages.remove(id.removePrefix("local:"))?.let {
                if (selected === it) selected = null
                memory.edit().remove("draft.${it.key}").apply()
            }
            publish()
        } else sendLocal(JSONObject().put("local", "close").put("instance", id))
    }

    private fun sendLocal(command: JSONObject): Boolean =
        handle != 0L && Native.send(handle, command.toString())

    private fun send(intent: String, page: Page? = selected): Boolean {
        if (
            page == null ||
                page.state.instance.isEmpty() ||
                !sendLocal(JSONObject(intent).put("instance", page.state.instance))
        ) {
            announce("Session unavailable or command queue full", page = page)
            return false
        }
        return true
    }

    fun prompt(text: String) {
        val page = selected ?: return
        if (text.isBlank() && page.state.attachments.isEmpty()) return
        if (send(Intents.prompt(text, page.state.attachments.map { it.blob }), page)) {
            update(page) { it.copy(attachments = emptyList(), draft = "") }
            memory.edit().putString("draft.${page.key}", "").apply()
        }
    }

    fun command(name: String, values: List<Pair<String, String>> = emptyList()) {
        val page = selected ?: return
        val line = page.state.draft.takeIf { it.trim().startsWith("/$name") }.orEmpty()
        val command = JSONObject(Intents.command(name, values)).put("text", line)
        if (send(command.toString(), page) && line.isNotEmpty()) draft("")
    }

    fun action(node: String, action: Action, fields: List<FieldValue> = emptyList()) {
        val page = selected ?: return
        if (action.id == "attachment.save") {
            if (page.saving) return
            page.saving = true
        }
        if (!send(Intents.action(node, action, fields), page)) page.saving = false
    }

    fun cancel() {
        send(Intents.cancel())
    }

    fun removeAttachment(index: Int) {
        update { it.copy(attachments = it.attachments.filterIndexed { at, _ -> at != index }) }
    }

    fun presentation(id: String, mode: String, variant: String? = null) {
        val choice = JSONObject().put("mode", mode)
        if (variant != null) choice.put("variant", variant)
        send(
            JSONObject().put("local", "presentation").put("id", id).put("choice", choice).toString()
        )
    }

    fun request(
        model: InputRequest,
        action: String,
        value: String,
        drafts: Map<String, String> = emptyMap(),
    ) {
        if (selected?.state?.connected != true) {
            announce("Request owner is unavailable")
            return
        }
        val fields = JSONObject(drafts)
        model.input?.let { fields.put(it, value) }
        send(
            JSONObject()
                .put("local", "request")
                .put("request", model.id)
                .put("generation", model.generation)
                .put("action", action)
                .put("fields", fields)
                .toString()
        )
    }

    fun requestDraft(id: String): String = selected?.requestDrafts?.get(id).orEmpty()

    fun requestDraft(id: String, value: String) {
        selected?.requestDrafts?.set(id, value)
    }

    fun dismissReport() {
        update { it.copy(report = null) }
    }

    fun dismissForm() {
        update { it.copy(form = null) }
    }

    fun submitForm(form: LocalForm, drafts: Map<String, String>) {
        if (form.daemon != null) {
            sendLocal(
                JSONObject()
                    .put("local", "daemon_invoke")
                    .put("daemon", form.daemon)
                    .put("scope", form.scope)
                    .put("command", form.action)
                    .put("drafts", JSONObject(drafts))
            )
            return
        }
        send(
            JSONObject()
                .put("local", "form")
                .put("action", form.action)
                .put("direct", form.direct)
                .put("drafts", JSONObject(drafts))
                .toString()
        )
    }

    fun complete(source: String, prefix: String) {
        val page = selected ?: return
        page.completionRequest = "$source:$prefix"
        update(page) { it.copy(completions = emptyList()) }
        send(
            JSONObject()
                .put("local", "complete")
                .put("source", source)
                .put("prefix", prefix)
                .put("request", page.completionRequest)
                .toString(),
            page,
        )
    }

    fun image(hash: String) {
        val page = selected ?: return
        if (
            !hash.matches(Regex("[0-9a-f]{64}")) ||
                hash in page.state.images ||
                page.requested.size >= 8 ||
                !page.requested.add(hash)
        )
            return
        viewModelScope.launch {
            val cached = withContext(Dispatchers.IO) { Native.cached(storage().absolutePath, hash) }
            if (cached != null) imageReady(page, hash, cached)
            else if (!send(JSONObject().put("local", "fetch").put("hash", hash).toString(), page))
                page.requested.remove(hash)
        }
    }

    private fun imageReady(page: Page, hash: String, path: String) {
        page.requested.remove(hash)
        update(page) {
            it.copy(
                images =
                    (it.images + (hash to path)).entries.toList().takeLast(128).associate { entry ->
                        entry.toPair()
                    }
            )
        }
    }

    fun attach(uri: Uri) {
        val page = selected ?: return
        if (page.state.uploading) return
        update(page) { it.copy(uploading = true) }
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val app = getApplication<Application>()
                    val resolver = app.contentResolver
                    var name = "attachment"
                    resolver
                        .query(
                            uri,
                            arrayOf(android.provider.OpenableColumns.DISPLAY_NAME),
                            null,
                            null,
                            null,
                        )
                        ?.use { if (it.moveToFirst()) name = it.getString(0) ?: name }
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
                                    require(total <= 32L * 1024 * 1024) {
                                        "Attachments are limited to 32 MiB"
                                    }
                                    output.write(buffer, 0, count)
                                }
                            }
                        } ?: error("Selected file could not be opened")
                        JSONObject()
                            .put("local", "upload")
                            .put("path", file.absolutePath)
                            .put("name", name)
                            .put("media", resolver.getType(uri) ?: "application/octet-stream")
                            .toString()
                    } catch (error: Exception) {
                        file.delete()
                        throw error
                    }
                }
            }
            result
                .onSuccess {
                    if (!send(it, page)) {
                        File(JSONObject(it).getString("path")).delete()
                        update(page) { state -> state.copy(uploading = false) }
                    }
                }
                .onFailure {
                    announce(it.message ?: "Upload failed", page = page)
                    update(page) { state -> state.copy(uploading = false) }
                }
        }
    }

    fun beginDownload(): String? {
        if (downloadOwner != null) return null
        val page = selected ?: return null
        val file = page.state.download ?: return null
        downloadOwner = page to file
        return file.name
    }

    fun saveDownload(uri: Uri?) {
        val (page, download) = downloadOwner ?: return
        downloadOwner = null
        update(page) { it.copy(download = null) }
        page.saving = false
        if (uri == null) return
        viewModelScope.launch {
            runCatching {
                    withContext(Dispatchers.IO) {
                        DocumentFiles.save(
                            getApplication<Application>().contentResolver,
                            File(download.path),
                            uri,
                        )
                    }
                }
                .onSuccess { announce("Saved ${download.name}", "info", page) }
                .onFailure { announce(it.message ?: "Save failed", page = page) }
        }
    }

    private fun pageKey(daemon: String, scope: JSONObject): String =
        daemon +
            ":" +
            scope.getJSONObject("id").getString("kind") +
            ":" +
            scope.getJSONObject("id").getString("id") +
            ":" +
            scope.getString("incarnation")

    private fun page(daemon: String, scope: JSONObject, title: String): Page {
        val key = pageKey(daemon, scope)
        check(pages.containsKey(key) || pages.size < 8) {
            "Close a local session before opening another"
        }
        return pages.getOrPut(key) {
            Page(
                key,
                daemon,
                scope,
                UiState(
                    daemon = daemon,
                    session =
                        Session(
                            scope.getJSONObject("id").optString("id"),
                            title,
                            emptyList(),
                            emptyList(),
                        ),
                    draft = memory.getString("draft.$key", "") ?: "",
                ),
            )
        }
    }

    private fun transaction(page: Page, documents: JSONObject) {
        Snapshot.withMutableSnapshot {
            documents.keys().forEach { slot ->
                val change = documents.getJSONObject(slot)
                val document = page.documents.getOrPut(slot) { Document() }
                when (change.getString("mode")) {
                    "reset" -> {
                        document.tree.reset(Wire.parseNode(change.getJSONObject("tree")))
                        document.streams.reset(change.getJSONArray("streams"))
                    }
                    "change" -> {
                        val ops = change.getJSONArray("ops")
                        if (ops.length() > 0)
                            document.tree.apply(JSONArray().put(JSONObject().put("ops", ops)))
                        if (change.optBoolean("reset_live")) document.streams.reset()
                        val live = change.getJSONArray("live")
                        for (index in 0 until live.length()) document.streams.apply(
                            live.getJSONObject(index)
                        )
                    }
                    "unavailable" -> {
                        page.documents.remove(slot)
                        announce(change.optString("message"), page = page)
                    }
                }
            }
            page.state =
                page.state.copy(
                    view = page.documents["conversation"]?.tree?.root,
                    panels =
                        page.documents
                            .filterKeys { it != "conversation" }
                            .mapNotNull { (id, doc) -> doc.tree.root?.let { id to it } }
                            .toMap(),
                )
            publish(page)
        }
    }

    private fun onEvent(event: JSONObject) {
        when (event.getString("kind")) {
            "daemon_commands" -> {
                val items = event.getJSONArray("commands")
                daemonCommands[event.getString("daemon")] =
                    (0 until items.length()).map { items.getString(it) }
                publish()
                return
            }
            "daemon_form" -> {
                val fields = event.getJSONArray("fields")
                update {
                    it.copy(
                        form =
                            LocalForm(
                                event.getString("action"),
                                (0 until fields.length()).map { index ->
                                    val pair = fields.getJSONArray(index)
                                    val field = pair.getJSONObject(1)
                                    FormField(
                                        pair.getString(0),
                                        field.optBoolean("optional"),
                                        field.getJSONObject("schema").getString("type") == "string",
                                    )
                                },
                                true,
                                event.getString("daemon"),
                                event.getJSONObject("scope"),
                                event.getJSONObject("drafts").let { values ->
                                    values.keys().asSequence().associateWith {
                                        values.getString(it)
                                    }
                                },
                            )
                    )
                }
                return
            }
            "daemon_report" -> {
                update {
                    it.copy(form = null, report = Wire.parseNode(event.getJSONObject("view")))
                }
                return
            }
            "archives" -> {
                val daemon = event.getString("daemon")
                if (archiveRequests[daemon] == event.getString("prefix")) {
                    archives[daemon] =
                        event.getJSONObject("items").getJSONArray("items").objects().map {
                            it.getString("value") to it.optString("label", it.getString("value"))
                        }
                    publish()
                }
                return
            }
            "ready" -> return
            "relationship" -> {
                val target = event.getString("target")
                val saved = JSONObject(memory.getString("relationshipTargets", "{}") ?: "{}")
                saved.remove(event.getString("daemon"))
                saved.put(event.getString("daemon"), target)
                while (saved.length() > 32) saved.remove(saved.keys().next())
                memory
                    .edit()
                    .putString("relationshipTargets", saved.toString())
                    .putString("ticket", target)
                    .apply()
                _state.value = _state.value.copy(ticket = target)
                return
            }
            "directory" -> {
                daemons =
                    event.getJSONArray("daemons").objects().map { daemon ->
                        DaemonRow(
                            daemon.getString("daemon"),
                            daemon.getString("freshness"),
                            daemon.getJSONArray("sessions").objects().map {
                                SessionRow(
                                    it.getString("title"),
                                    JSONObject()
                                        .put(
                                            "id",
                                            JSONObject()
                                                .put("kind", "session")
                                                .put("id", it.getString("id")),
                                        )
                                        .put("incarnation", it.getString("incarnation")),
                                    it.optString("summary"),
                                    it.optJSONArray("requests")?.objects()?.mapNotNull { target ->
                                        val request = target.getJSONObject("request")
                                        val id = request.optString("id")
                                        if (id.isEmpty() || !request.has("generation")) null
                                        else
                                            RequestTarget(
                                                daemon.getString("daemon"),
                                                target.getJSONObject("scope"),
                                                id,
                                                request.getLong("generation"),
                                                request.optString(
                                                    "title",
                                                    request.optString("kind", "Pending request"),
                                                ),
                                                target.optBoolean("available") &&
                                                    daemon.getString("freshness") == "current",
                                            )
                                    } ?: emptyList(),
                                )
                            },
                            daemon.optJSONArray("work")?.objects()?.map { work ->
                                WorkRow(
                                    work.getString("id"),
                                    work.getJSONObject("scope"),
                                    work.getString("state"),
                                    work.getBoolean("blocking"),
                                    work.getBoolean("available"),
                                )
                            } ?: emptyList(),
                        )
                    }
                instances =
                    event.getJSONArray("instances").objects().map {
                        InstanceRow(
                            it.getString("instance"),
                            it.getString("daemon"),
                            it.getString("title"),
                            it.getBoolean("available"),
                        )
                    }
                pendingRequest?.let { target ->
                    if (
                        daemons
                            .flatMap { it.sessions }
                            .flatMap { it.requests }
                            .none {
                                it.available &&
                                    it.daemon == target.daemon &&
                                    it.id == target.id &&
                                    it.generation == target.generation &&
                                    pageKey(it.daemon, it.scope) ==
                                        pageKey(target.daemon, target.scope)
                            }
                    ) {
                        pendingRequest = null
                        announce("Request source changed or is no longer pending")
                    }
                }
                publish()
                restoreSelection?.let { (daemon, scope) ->
                    if (
                        daemons.any {
                            it.id == daemon &&
                                it.freshness == "current" &&
                                it.sessions.any { session ->
                                    pageKey(daemon, session.scope) == pageKey(daemon, scope)
                                }
                        }
                    ) {
                        restoreSelection = null
                        select(daemon, scope)
                    }
                }
                return
            }
            "instance",
            "selected" -> {
                val id = event.getString("instance")
                val page =
                    page(
                        event.getString("daemon"),
                        event.getJSONObject("scope"),
                        event.getString("title"),
                    )
                page.state = page.state.copy(instance = id)
                nativePages[id] = page
                if (event.getString("kind") == "selected") {
                    selected = page
                    publish()
                }
                return
            }
            "instance_closed" -> {
                nativePages.remove(event.getString("instance"))?.let { page ->
                    pages.remove(page.key)
                    memory.edit().remove("draft.${page.key}").apply()
                    if (selected === page) selected = null
                }
                publish()
                return
            }
            "cached" -> {
                val page =
                    page(
                        event.getString("daemon"),
                        event.getJSONObject("scope"),
                        event.optString("title"),
                    )
                page.state =
                    page.state.copy(
                        instance = "local:" + page.key,
                        phase = Phase.Closed,
                        message = "Offline · saved canonical content",
                    )
                if (selected == null) {
                    selected = page
                    if (restoreCached) restoreSelection = page.daemon to page.scope
                }
                transaction(page, event.getJSONObject("documents"))
                return
            }
        }
        val page =
            event.optString("instance").takeIf { it.isNotEmpty() }?.let { nativePages[it] }
                ?: run {
                    if (event.optString("kind") in listOf("notice", "fault"))
                        announce(event.optString("text", event.optString("message")))
                    return
                }
        when (event.getString("kind")) {
            "form" -> {
                val fields = event.getJSONArray("fields")
                update(page) {
                    it.copy(
                        form =
                            LocalForm(
                                event.getString("action"),
                                (0 until fields.length()).map { at ->
                                    val pair = fields.getJSONArray(at)
                                    val field = pair.getJSONObject(1)
                                    FormField(
                                        pair.getString(0),
                                        field.optBoolean("optional"),
                                        field.getJSONObject("schema").getString("type") == "string",
                                    )
                                },
                                event.optBoolean("direct"),
                            )
                    )
                }
            }
            "state" ->
                update(page) {
                    it.copy(
                        phase =
                            if (event.getString("state") == "connected") Phase.Connected
                            else Phase.Closed,
                        message = event.optString("message"),
                    )
                }
            "session" ->
                update(page) {
                    it.copy(session = Wire.parseSession(event.getJSONObject("session")))
                }
            "composition" -> {
                page.composition = event.getLong("composition")
                page.render = 0
                page.requiredRender = 0
                page.recovering = false
                page.recoveryAttempts = 0
                val slots =
                    event.getJSONArray("slots").let { array ->
                        (0 until array.length()).map { array.getString(it) }.toSet()
                    }
                page.documents.keys.retainAll(slots)
                val prefs = event.getJSONObject("preferences")
                val capabilities =
                    event.getJSONArray("capabilities").let { array ->
                        (0 until array.length()).map { array.getString(it) }.toSet()
                    }
                val choices =
                    event.getJSONArray("catalog").objects().map {
                        val id = it.getString("id")
                        val choice = prefs.optJSONObject(id)
                        PresentationChoice(
                            id,
                            it.getString("title"),
                            choice?.let { value ->
                                if (value.optString("mode") == "variant") value.getString("variant")
                                else value.getString("mode")
                            } ?: if (id in listOf("conversation", "status")) "auto" else "hidden",
                            it.getJSONArray("variants")
                                .objects()
                                .filter { variant ->
                                    variant.getJSONArray("requirements").let { required ->
                                        (0 until required.length()).all {
                                            required.getString(it) in capabilities
                                        }
                                    }
                                }
                                .map { variant -> variant.getString("id") },
                        )
                    }
                update(page) {
                    it.copy(
                        presentations = choices,
                        panels = it.panels.filterKeys { id -> id in slots },
                    )
                }
            }
            "transaction" -> {
                if (event.getLong("composition") != page.composition) return
                val render = event.getLong("render")
                if (render < page.requiredRender) return
                page.render = render
                val documents = event.getJSONObject("documents")
                if (
                    page.recovering &&
                        documents.keys().asSequence().any {
                            documents.getJSONObject(it).getString("mode") == "change"
                        }
                )
                    return
                transaction(page, documents)
                if (documents.optJSONObject("conversation")?.optString("mode") == "reset") {
                    page.recovering = false
                    page.recoveryAttempts = 0
                    update(page) { it.copy(renderProblem = null) }
                }
            }
            "request" -> {
                val id = event.getString("id")
                val model = event.optJSONObject("model")
                pendingRequest?.let { target ->
                    if (
                        model == null &&
                            target.id == id &&
                            page.key == pageKey(target.daemon, target.scope)
                    ) {
                        pendingRequest = null
                        announce("Request is no longer pending", page = page)
                    }
                }
                val generation = model?.getLong("generation")
                page.requestDrafts.keys.removeAll {
                    it.startsWith(id + ":") &&
                        (generation == null ||
                            (it != "$id:$generation" && !it.startsWith("$id:$generation:")))
                }
                val input = model?.optJSONObject("input")
                val request = model?.let {
                    InputRequest(
                        id,
                        it.getLong("generation"),
                        it.getString("title"),
                        Wire.parseNode(it.getJSONObject("body")),
                        input?.getString("id"),
                        input?.getString("label") ?: "",
                        input?.optBoolean("secret") ?: false,
                        it.getJSONArray("actions").objects().map { action ->
                            RequestAction(action.getString("id"), action.getString("label"))
                        },
                        it.optJSONObject("form")?.let { form ->
                            val fields = form.getJSONObject("input").getJSONObject("fields")
                            fields
                                .keys()
                                .asSequence()
                                .map { id ->
                                    val field = fields.getJSONObject(id)
                                    FormField(
                                        id,
                                        field.optBoolean("optional"),
                                        field.getJSONObject("schema").getString("type") == "string",
                                        form
                                            .optJSONObject("fields")
                                            ?.optJSONObject(id)
                                            ?.optString("label", id) ?: id,
                                    )
                                }
                                .toList()
                        } ?: emptyList(),
                    )
                }
                update(page) {
                    it.copy(
                        requests =
                            it.requests.filterNot { old -> old.id == id } + listOfNotNull(request)
                    )
                }
            }
            "report" ->
                update(page) { it.copy(report = Wire.parseNode(event.getJSONObject("view"))) }
            "rejected" -> {
                val text = event.optString("text").takeUnless { it == "null" } ?: ""
                if (text.isNotEmpty() && page.state.draft.isEmpty())
                    update(page) { it.copy(draft = text) }
                else if (text.isNotEmpty()) announce("Unsent input retained: $text", page = page)
                announce(event.optString("message"), page = page)
                page.saving = false
            }
            "blob" -> imageReady(page, event.getString("hash"), event.getString("path"))
            "blob_failed" -> {
                page.requested.remove(event.getString("hash"))
                update(page) {
                    it.copy(
                        imageErrors =
                            it.imageErrors + (event.getString("hash") to event.optString("text"))
                    )
                }
                page.saving = false
            }
            "uploaded" ->
                update(page) {
                    it.copy(
                        uploading = false,
                        attachments =
                            it.attachments +
                                PendingAttachment(
                                    event.optString("name"),
                                    event.getJSONObject("blob"),
                                ),
                    )
                }
            "upload_failed" -> {
                update(page) { it.copy(uploading = false) }
                announce(event.optString("text"), page = page)
            }
            "download" ->
                update(page) {
                    it.copy(
                        download =
                            DownloadFile(
                                event.getString("path"),
                                File(event.optString("name", "attachment"))
                                    .name
                                    .filter { char -> char.code >= 32 }
                                    .take(160)
                                    .ifBlank { "attachment" },
                            )
                    )
                }
            "notice" -> announce(event.optString("text"), event.optString("level", "info"), page)
            "completion" ->
                if (event.optString("request") == page.completionRequest) {
                    val items = event.getJSONObject("items").getJSONArray("items")
                    update(page) {
                        it.copy(
                            completions =
                                items.objects().map { item ->
                                    item.optString("value") to
                                        item.optString("label", item.optString("value"))
                                }
                        )
                    }
                }
        }
    }

    override fun onCleared() {
        pause()
        super.onCleared()
    }
}

internal fun JSONArray.objects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }
