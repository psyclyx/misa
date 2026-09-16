package org.misa.app

import android.app.Activity
import android.app.Instrumentation
import android.content.ContentValues
import android.graphics.Bitmap
import android.os.Bundle
import android.provider.MediaStore
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

/** Runs the actual JNI/iroh/file boundary in an emulator; no test-only transport. */
class ClientInstrumentation : Instrumentation() {
    private var options = Bundle()

    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        options = arguments ?: Bundle()
        start()
    }

    override fun onStart() {
        val result = Bundle()
        try {
            val ticket =
                requireNotNull(options.getString("ticket")) { "provide -e ticket <daemon ticket>" }
            incrementalWork()
            exercise(ticket)
            lifecycle(ticket)
            val second = options.getString("ticket2")?.takeIf { it.isNotEmpty() }
            second?.let { relationships(ticket, it) }
            if (options.getString("plugin") == "true") contributedForm(ticket)
            second?.let { workspaceUiState(ticket, it) }
            result.putString(
                "stream",
                "\nPASS: incremental canonical/live updates, persistent reconnect, offline canonical cache, blob upload/fetch, directed save destination${if(second!=null) ", two daemon identities, private tool and credential workflows" else ""}\n",
            )
            result.putInt(
                "tests",
                (if (second == null) 6 else 9) +
                    (if (options.getString("plugin") == "true") 1 else 0),
            )
            finish(Activity.RESULT_OK, result)
        } catch (error: Throwable) {
            result.putString("stream", "\nFAIL: ${error.stackTraceToString()}\n")
            finish(Activity.RESULT_CANCELED, result)
        }
    }

    private inner class Connection(ticket: String, directory: File) : AutoCloseable {
        val events = LinkedBlockingQueue<JSONObject>()
        private val latest = java.util.concurrent.ConcurrentHashMap<String, JSONObject>()
        private val trees = mutableMapOf<String, ViewTree>()
        private val streams = mutableMapOf<String, LiveStreams>()
        val handle =
            Native.connect(
                ticket,
                directory.absolutePath,
                Listener {
                    val wake = JSONObject(it)
                    while (true) {
                        val raw = Native.poll(wake.getLong("handle")) ?: break
                        val event = JSONObject(raw)
                        if (event.optString("kind") == "transaction") {
                            val instance = event.getString("instance")
                            val tree = trees.getOrPut(instance) { ViewTree() }
                            val live = streams.getOrPut(instance) { LiveStreams() }
                            val change =
                                event.getJSONObject("documents").optJSONObject("conversation")
                            if (change != null) {
                                androidx.compose.runtime.snapshots.Snapshot.withMutableSnapshot {
                                    when (change.optString("mode")) {
                                        "reset" -> {
                                            tree.reset(Wire.parseNode(change.getJSONObject("tree")))
                                            live.reset(change.getJSONArray("streams"))
                                        }
                                        "change" -> {
                                            tree.apply(
                                                JSONArray()
                                                    .put(
                                                        JSONObject()
                                                            .put("ops", change.getJSONArray("ops"))
                                                    )
                                            )
                                            if (change.getBoolean("reset_live")) live.reset()
                                            change
                                                .getJSONArray("live")
                                                .objects()
                                                .forEach(live::apply)
                                        }
                                    }
                                }
                                tree.root?.let { root ->
                                    val view =
                                        JSONObject()
                                            .put("kind", "view")
                                            .put("instance", instance)
                                            .put("view", testJson(root))
                                    latest["view"] = view
                                    events.offer(view)
                                }
                            }
                        } else {
                            latest[event.getString("kind")] = event
                            events.offer(event)
                        }
                    }
                },
            )

        init {
            check(handle != 0L)
        }

        fun send(json: String) {
            check(Native.send(handle, json)) { "connection refused local input" }
        }

        fun waitFor(kind: String, accept: (JSONObject) -> Boolean = { true }): JSONObject {
            latest[kind]?.let { if (accept(it)) return it }
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(45)
            val seen = mutableListOf<String>()
            while (System.nanoTime() < deadline) {
                val event = events.poll(1, TimeUnit.SECONDS) ?: continue
                seen.add(
                    event.optString("kind") +
                        ":" +
                        event.optString("text", event.optString("message"))
                )
                check(event.optString("kind") != "fault") { event.toString() }
                if (event.optString("kind") == kind && accept(event)) return event
            }
            error("timed out waiting for $kind; saw ${seen.takeLast(30)}")
        }

        override fun close() {
            Native.disconnect(handle)
        }
    }

    private fun nodes(root: JSONObject): List<JSONObject> = buildList {
        add(root)
        val children = root.optJSONArray("children") ?: JSONArray()
        for (i in 0 until children.length()) addAll(nodes(children.getJSONObject(i)))
    }

    private fun settled(view: JSONObject): Set<String> =
        nodes(view)
            .filter {
                it.optString("role") == "message.assistant" &&
                    it.optString("state") in listOf("done", "failed", "cancelled")
            }
            .map { it.optString("id") }
            .toSet()

    private fun Connection.turn(
        text: String,
        before: JSONObject,
        attachments: List<JSONObject> = emptyList(),
    ): JSONObject {
        val previous = settled(before)
        send(Intents.prompt(text, attachments))
        return waitFor("view") { event ->
                val view = event.getJSONObject("view")
                view.toString().contains(text) &&
                    settled(view).any { it !in previous } &&
                    !view.toString().contains("turn.cancel")
            }
            .getJSONObject("view")
    }

    // The test harness materializes JSON for assertions; production rendering keeps Node
    // references.
    private fun testJson(node: Node): JSONObject =
        JSONObject()
            .put("id", node.id)
            .put("role", node.role)
            .put("state", node.state)
            .put("children", JSONArray(node.children.map(::testJson)))
            .put("text", (node.shape as? Shape.Text)?.spans?.joinToString("") { it.text } ?: "")
            .put("actions", JSONArray(node.actions.map { JSONObject().put("id", it.id) }))

    private fun incrementalWork() {
        fun tree(size: Int): ViewTree =
            ViewTree().apply {
                reset(
                    Node(
                        "root",
                        "root",
                        null,
                        null,
                        Shape.Section,
                        emptyList(),
                        (0 until size).map {
                            Node(
                                "msg.$it",
                                "message.user",
                                null,
                                "done",
                                Shape.Text(listOf(Span("old", "plain"))),
                                emptyList(),
                                emptyList(),
                            )
                        },
                    )
                )
            }
        fun replace(tree: ViewTree) {
            tree.apply(
                JSONArray(
                    """[{"ops":[{"op":"replace","id":"msg.0","node":{"id":"msg.0","role":"message.user","kind":{"shape":"text","spans":[{"text":"new"}]}}}]}]"""
                )
            )
        }
        val small = tree(1)
        val large = tree(1000)
        val untouched = large.root!!.children.last()
        replace(small)
        replace(large)
        check(small.touched == 2 && large.touched == 2)
        check(large.root!!.children.last() === untouched)
        val canonical = large.root
        val streams = LiveStreams()
        streams.apply(
            JSONObject(
                """{"update":"current","stream":{"id":"live.text","role":"message.assistant","text":"é"}}"""
            )
        )
        streams.apply(JSONObject("""{"update":"append","id":"live.text","offset":2,"text":"!"}"""))
        check(
            streams.values.getValue("live.text").text == "é!" &&
                streams.values.getValue("live.text").bytes == 3
        )
        streams.apply(JSONObject("""{"update":"append","id":"live.text","offset":3,"text":"🙂"}"""))
        check(streams.values.getValue("live.text").bytes == 7)
        check(
            runCatching {
                    streams.apply(
                        JSONObject(
                            """{"update":"append","id":"live.text","offset":1,"text":"bad"}"""
                        )
                    )
                }
                .isFailure
        )
        check(large.root === canonical && large.touched == 2)
    }

    private fun exercise(ticket: String) {
        val root =
            File(targetContext.filesDir, "instrumentation-client").apply {
                deleteRecursively()
                mkdirs()
            }
        val peer =
            File(targetContext.filesDir, "instrumentation-peer").apply {
                deleteRecursively()
                mkdirs()
            }
        try {
            var saved: JSONObject
            Connection(ticket, root).use { client ->
                val initial = client.waitFor("view").getJSONObject("view")
                saved = client.turn("android-first", initial)
                val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
                while (
                    root.listFiles().orEmpty().none {
                        it.name.startsWith("view-") &&
                            it.name.endsWith(".json") &&
                            it.readText().contains("android-first")
                    }
                ) {
                    check(System.nanoTime() < deadline) { "canonical checkpoint was not saved" }
                    Thread.sleep(20)
                }
            }
            val identity = File(root, "identity").readBytes()
            val snapshot =
                root
                    .listFiles()!!
                    .filter { it.name.startsWith("view-") }
                    .first { it.readText().contains("android-first") }
                    .readText()
            check(JSONObject(snapshot).has("resume"))
            Connection(ticket, peer).use { other ->
                val initial = other.waitFor("view").getJSONObject("view")
                other.turn("android-while-away", initial)
            }
            Connection(ticket, root).use { client ->
                saved =
                    client
                        .waitFor("view") { it.toString().contains("android-while-away") }
                        .getJSONObject("view")
                check(File(root, "identity").readBytes().contentEquals(identity))
                val pictureSource =
                    Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888).apply {
                        eraseColor(android.graphics.Color.RED)
                    }
                val png =
                    ByteArrayOutputStream().use { output ->
                        pictureSource.compress(Bitmap.CompressFormat.PNG, 100, output)
                        output.toByteArray()
                    }
                pictureSource.recycle()
                val upload =
                    File(targetContext.cacheDir, "instrumentation-upload.png").apply {
                        writeBytes(png)
                    }
                client.send(
                    JSONObject()
                        .put("local", "upload")
                        .put("path", upload.absolutePath)
                        .put("name", "picture.png")
                        .put("media", "image/png")
                        .toString()
                )
                val blob = client.waitFor("uploaded").getJSONObject("blob")
                check(!upload.exists())
                saved = client.turn("android-attachment", saved, listOf(blob))
                check(saved.toString().contains("attachment.save"))
                client.send(
                    JSONObject()
                        .put("local", "fetch")
                        .put("hash", blob.getString("hash"))
                        .toString()
                )
                val fetched = client.waitFor("blob")
                check(File(fetched.getString("path")).readBytes().contentEquals(png))
                val picture =
                    requireNotNull(
                        android.graphics.BitmapFactory.decodeFile(fetched.getString("path"))
                    )
                check(picture.width == 1 && picture.height == 1)
                picture.recycle()
                val attachment =
                    nodes(saved).first { node ->
                        val actions = node.optJSONArray("actions") ?: JSONArray()
                        (0 until actions.length()).any {
                            actions.getJSONObject(it).optString("id") == "attachment.save"
                        }
                    }
                client.send(
                    JSONObject()
                        .put("intent", "action")
                        .put("node", attachment.getString("id"))
                        .put("action", "attachment.save")
                        .toString()
                )
                val download = client.waitFor("download")
                val resolver = targetContext.contentResolver
                val values =
                    ContentValues().apply {
                        put(MediaStore.Downloads.DISPLAY_NAME, "misa-instrumentation.png")
                        put(MediaStore.Downloads.MIME_TYPE, "image/png")
                        put(MediaStore.Downloads.RELATIVE_PATH, "Download/Misa-tests")
                    }
                val destination =
                    requireNotNull(
                        resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                    )
                try {
                    DocumentFiles.save(resolver, File(download.getString("path")), destination)
                    check(
                        resolver
                            .openInputStream(destination)!!
                            .use { it.readBytes() }
                            .contentEquals(png)
                    )
                } finally {
                    resolver.delete(destination, null, null)
                }
            }
            val offline =
                ticket.substringBefore('@') + "@10.0.2.2:1:" + ticket.substringAfterLast(':')
            Connection(offline, root).use { client ->
                val cached = client.waitFor("cached")
                check(
                    cached
                        .getJSONObject("documents")
                        .getJSONObject("conversation")
                        .getJSONObject("tree")
                        .toString()
                        .contains("android-attachment")
                )
                check(
                    cached
                        .getJSONObject("documents")
                        .getJSONObject("conversation")
                        .getJSONArray("streams")
                        .length() == 0
                )
            }
        } finally {
            root.deleteRecursively()
            peer.deleteRecursively()
        }
    }

    private fun lifecycle(ticket: String) {
        val directory =
            File(targetContext.filesDir, "instrumentation-lifecycle").apply {
                deleteRecursively()
                mkdirs()
            }
        try {
            Connection(ticket, directory).use { client ->
                client.waitFor("view")
                val daemon = client.waitFor("selected").getString("daemon")
                client.waitFor("directory") { event ->
                    event.getJSONArray("daemons").objects().any { owner ->
                        owner.getJSONArray("sessions").objects().any {
                            it.optString("summary").contains("tokens")
                        }
                    }
                }
                fun archive() {
                    client.send(
                        JSONObject()
                            .put("local", "archives")
                            .put("daemon", daemon)
                            .put("prefix", "")
                            .toString()
                    )
                }
                archive()
                val before =
                    client
                        .waitFor("archives")
                        .getJSONObject("items")
                        .getJSONArray("items")
                        .objects()
                        .map { it.getString("value") }
                        .toSet()
                fun invoke(command: String, input: JSONObject) {
                    client.send(
                        JSONObject()
                            .put("local", "lifecycle")
                            .put("daemon", daemon)
                            .put("command", command)
                            .put("input", input)
                            .toString()
                    )
                }
                invoke("daemon.session.create", JSONObject().put("id", "android-created"))
                val created =
                    client.waitFor("selected") { it.optString("title") == "android-created" }
                val instance = created.getString("instance")
                val first =
                    client
                        .waitFor("view") { it.getString("instance") == instance }
                        .getJSONObject("view")
                client.turn("android-resumable-history", first)
                val scope = created.getJSONObject("scope")
                invoke(
                    "daemon.session.close",
                    JSONObject()
                        .put("id", "android-created")
                        .put("incarnation", scope.getString("incarnation")),
                )
                client.waitFor("directory") { event ->
                    event.getJSONArray("daemons").objects().all { owner ->
                        owner.getJSONArray("sessions").objects().none {
                            it.getString("id") == "android-created"
                        }
                    }
                }
                archive()
                val archive =
                    client.waitFor("archives") { event ->
                        event.getJSONObject("items").getJSONArray("items").objects().any {
                            it.getString("value") !in before
                        }
                    }
                val conversation =
                    archive
                        .getJSONObject("items")
                        .getJSONArray("items")
                        .objects()
                        .first { it.getString("value") !in before }
                        .getString("value")
                invoke(
                    "daemon.session.resume",
                    JSONObject().put("id", "android-created").put("conversation", conversation),
                )
                val resumed =
                    client.waitFor("selected") {
                        it.optString("title") == "android-created" &&
                            it.getJSONObject("scope").getString("incarnation") !=
                                scope.getString("incarnation")
                    }
                client.waitFor("view") {
                    it.getString("instance") == resumed.getString("instance") &&
                        it.toString().contains("android-resumable-history")
                }
            }
        } finally {
            directory.deleteRecursively()
        }
    }

    private fun contributedForm(ticket: String) {
        val directory =
            File(targetContext.filesDir, "instrumentation-contributed").apply {
                deleteRecursively()
                mkdirs()
            }
        try {
            Connection(ticket, directory).use { client ->
                val instance = client.waitFor("view").getString("instance")
                client.send(
                    JSONObject(Intents.command("pet.ask-feed", emptyList()))
                        .put("instance", instance)
                        .toString()
                )
                client.waitFor("form") {
                    it.optString("action") == "pet.ask-feed" && it.optBoolean("direct")
                }
                client.send(
                    JSONObject()
                        .put("local", "form")
                        .put("instance", instance)
                        .put("action", "pet.ask-feed")
                        .put("direct", true)
                        .put("drafts", JSONObject())
                        .toString()
                )
                val request =
                    client
                        .waitFor("request") {
                            it.optJSONObject("model")?.optJSONObject("form") != null
                        }
                        .getJSONObject("model")
                check(
                    request
                        .getJSONObject("form")
                        .getJSONObject("fields")
                        .getJSONObject("amount")
                        .getString("label") == "Treats (1–10)"
                )
                fun submit(amount: String) =
                    client.send(
                        JSONObject()
                            .put("local", "request")
                            .put("instance", instance)
                            .put("request", request.getString("id"))
                            .put("generation", request.getLong("generation"))
                            .put("action", "resolve")
                            .put("fields", JSONObject().put("amount", amount))
                            .toString()
                    )
                submit("bad")
                client.waitFor("rejected") { it.optString("message").contains("amount") }
                submit("3")
                client.waitFor("request") {
                    it.optString("id") == request.getString("id") &&
                        it.optJSONObject("model") == null
                }
                client.waitFor("notice") { it.optString("text") == "Operation succeeded" }
                client.send(
                    JSONObject()
                        .put("local", "form")
                        .put("instance", instance)
                        .put("action", "pet.ask-feed")
                        .put("direct", true)
                        .put("drafts", JSONObject())
                        .toString()
                )
                val cancelled =
                    client
                        .waitFor("request") { event ->
                            event.optJSONObject("model")?.let {
                                it.optJSONObject("form") != null &&
                                    it.getString("id") != request.getString("id")
                            } == true
                        }
                        .getJSONObject("model")
                client.send(
                    JSONObject()
                        .put("local", "request")
                        .put("instance", instance)
                        .put("request", cancelled.getString("id"))
                        .put("generation", cancelled.getLong("generation"))
                        .put("action", "cancel")
                        .put("fields", JSONObject().put("amount", "invalid populated draft"))
                        .toString()
                )
                client.waitFor("request") {
                    it.optString("id") == cancelled.getString("id") &&
                        it.optJSONObject("model") == null
                }
                client.waitFor("notice") { it.optString("text") == "Operation cancelled" }
            }
        } finally {
            directory.deleteRecursively()
        }
    }

    private fun <T> main(block: () -> T): T {
        var result: Result<T>? = null
        runOnMainSync { result = runCatching(block) }
        return result!!.getOrThrow()
    }

    private fun workspaceUiState(first: String, second: String) {
        val app = targetContext.applicationContext as android.app.Application
        val failNext = java.util.concurrent.atomic.AtomicBoolean(false)
        val failed = java.util.concurrent.atomic.AtomicBoolean(false)
        val latestComposition = java.util.concurrent.atomic.AtomicLong(0)
        val failFrom = java.util.concurrent.atomic.AtomicLong(Long.MAX_VALUE)
        val model = main {
            MisaViewModel(app) { raw ->
                val event = JSONObject(raw)
                if (event.optString("kind") == "composition")
                    latestComposition.set(event.getLong("composition"))
                val change = event.optJSONObject("documents")?.optJSONObject("conversation")
                if (
                    event.optString("kind") == "transaction" &&
                        event.optLong("composition") >= failFrom.get() &&
                        change?.optString("mode") == "reset" &&
                        failNext.compareAndSet(true, false)
                ) {
                    val tree = change.getJSONObject("tree")
                    tree.put("id", tree.getJSONArray("children").getJSONObject(0).getString("id"))
                    failed.set(true)
                }
                event
            }
        }
        fun awaitState(accept: (UiState) -> Boolean): UiState {
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(45)
            while (System.nanoTime() < deadline) {
                val state = model.state.value
                if (accept(state)) return state
                Thread.sleep(20)
            }
            error(
                "Timed out waiting for local workspace: ${model.state.value.message}, ${model.state.value.notices}"
            )
        }
        try {
            main { model.connect(first) }
            val initial = awaitState { it.connected && it.panels.containsKey("status") }
            main { model.daemonCommands(initial.daemon) }
            awaitState { state ->
                state.daemonCommands[initial.daemon]
                    .orEmpty()
                    .containsAll(listOf("operation.cancel", "daemon.work.forget"))
            }
            main { model.daemonForm(initial.daemon, "daemon.work.forget") }
            val daemonForm = awaitState { it.form?.daemon == initial.daemon }.form!!
            check(daemonForm.fields.map { it.id }.containsAll(listOf("operation", "generation")))
            main {
                model.submitForm(
                    daemonForm,
                    mapOf("operation" to "android-nonexistent-operation", "generation" to "1"),
                )
            }
            awaitState { it.notices.any { notice -> notice.text.contains("Rejected:") } }
            main { model.dismissForm() }
            check(initial.session!!.sources.any { it.id == "providers" })
            check(
                initial.session.commands.none {
                    it.id == "credentials.resolve" ||
                        it.id == "input.resolve" ||
                        it.id == "input.cancel"
                }
            )
            main { model.complete("providers", "anth") }
            awaitState { it.completions.any { choice -> choice.first == "anthropic" } }
            main {
                model.draft("kept while reading usage")
                model.command("usage")
            }
            val reported = awaitState { it.report != null }
            fun typed(node: Node): Boolean = node.shape is Shape.Fact || node.children.any(::typed)
            check(typed(reported.report!!) && reported.draft == "kept while reading usage")
            main { model.dismissReport() }
            val beforeFailure = model.state.value.view!!.id
            failFrom.set(latestComposition.get() + 1)
            failNext.set(true)
            main { model.presentation("conversation", "auto") }
            awaitState {
                failed.get() &&
                    it.renderProblem == null &&
                    it.view?.id == beforeFailure &&
                    it.notices.any { notice ->
                        notice.text.startsWith("Presentation cache failed:")
                    }
            }
            val field = initial.panels.getValue("status").children.first().id
            main {
                model.theme("light")
                model.statusField(field, false)
            }
            val restored = main { MisaViewModel(app) }
            check(
                restored.state.value.theme == "light" && field in restored.state.value.hiddenStatus
            )
            main { model.statusField(field, true) }
            if (options.getString("plugin") == "true") {
                check("plugin.pet.companion" !in initial.panels)
                check(initial.session.commands.any { it.id == "pet.ask-feed" })
                main { model.presentation("plugin.pet.companion", "auto") }
                awaitState { it.panels["plugin.pet.companion"]?.shape is Shape.Meter }
                main { model.presentation("plugin.pet.companion", "variant", "portable") }
                awaitState { it.panels["plugin.pet.companion"]?.shape is Shape.Status }
                main { model.presentation("plugin.pet.companion", "hidden") }
                awaitState { "plugin.pet.companion" !in it.panels }
            }
            main { model.command("login", listOf("provider" to "anthropic")) }
            val owner = awaitState { it.requests.any { request -> request.secret } }
            val request = owner.requests.first { it.secret }
            main { model.connect(second) }
            awaitState { it.connected && it.daemon != owner.daemon }
            val directory = awaitState {
                it.daemons
                    .flatMap { daemon -> daemon.sessions }
                    .flatMap { session -> session.requests }
                    .any { target ->
                        target.daemon == owner.daemon && target.id == request.id && target.available
                    }
            }
            val target =
                directory.daemons
                    .flatMap { it.sessions }
                    .flatMap { it.requests }
                    .first { it.daemon == owner.daemon && it.id == request.id }
            main { model.navigateRequest(target.copy(available = false)) }
            check(model.state.value.daemon != owner.daemon)
            main { model.navigateRequest(target) }
            val focused = awaitState { it.daemon == owner.daemon && it.requestFocus == request.id }
            check(
                focused.requests.any { it.id == request.id && it.generation == target.generation }
            )
            main {
                model.consumeRequestFocus()
                model.request(request, "cancel", "unused invalid draft")
            }
            awaitState { it.requests.none { it.id == request.id } }
            if (options.getString("restart") == "true") {
                val oldInstance = model.state.value.instance
                val control = File(targetContext.filesDir, "restart-control")
                val replacement = File(targetContext.filesDir, "restart-ticket")
                replacement.delete()
                try {
                    control.writeText("stop")
                    val stale = awaitState { state ->
                        state.daemons.any { it.id == owner.daemon && it.freshness != "current" } &&
                            state.daemons.any {
                                it.id != owner.daemon && it.freshness == "current"
                            } &&
                            !state.connected
                    }
                    check(stale.instance == oldInstance && stale.view != null)
                    control.writeText("start")
                    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(45)
                    while (!replacement.exists() || replacement.length() == 0L) {
                        check(System.nanoTime() < deadline) { "Harness did not restart daemon" }
                        Thread.sleep(20)
                    }
                    val refreshed = replacement.readText().trim()
                    // Refresh the address of the relationship, without choosing its session hint.
                    main { model.connect(refreshed.removePrefix("misa:").substringBeforeLast(":")) }
                    val restarted = awaitState { state ->
                        state.daemons.any { daemon ->
                            daemon.id == owner.daemon &&
                                daemon.freshness == "current" &&
                                daemon.sessions.any { session ->
                                    session.scope.getJSONObject("id").getString("id") ==
                                        target.scope.getJSONObject("id").getString("id") &&
                                        session.scope.getString("incarnation") !=
                                            target.scope.getString("incarnation")
                                }
                        }
                    }
                    check(restarted.instance == oldInstance && !restarted.connected)
                    val targets =
                        JSONObject(
                            app.getSharedPreferences("client", 0)
                                .getString("relationshipTargets", "{}")!!
                        )
                    check(
                        targets.getString(owner.daemon) ==
                            refreshed.removePrefix("misa:").substringBeforeLast(":")
                    )
                    check(restarted.instances.any { it.id == oldInstance && !it.available })
                    val fresh =
                        restarted.daemons
                            .first { it.id == owner.daemon }
                            .sessions
                            .first {
                                it.scope.getJSONObject("id").getString("id") ==
                                    target.scope.getJSONObject("id").getString("id")
                            }
                    main { model.select(owner.daemon, fresh.scope) }
                    awaitState { it.connected && it.instance != oldInstance }
                    main { model.selectInstance(oldInstance) }
                    val retained = awaitState { it.instance == oldInstance }
                    check(!retained.connected && retained.draft == "kept while reading usage")
                    main { model.select(owner.daemon, fresh.scope) }
                    awaitState { it.connected && it.instance != oldInstance }
                } finally {
                    control.delete()
                    replacement.delete()
                }
            }
        } finally {
            main { model.pause() }
        }
    }

    private fun relationships(first: String, second: String) {
        val directory =
            File(targetContext.filesDir, "instrumentation-workspace").apply {
                deleteRecursively()
                mkdirs()
            }
        try {
            Connection(first, directory).use { client ->
                val initial = client.waitFor("view")
                val firstInstance = initial.getString("instance")
                client.send(JSONObject().put("local", "connect").put("target", second).toString())
                val other = client.waitFor("view") { it.getString("instance") != firstInstance }
                val secondInstance = other.getString("instance")
                client.send(
                    JSONObject(Intents.prompt("android-private-tool"))
                        .put("instance", secondInstance)
                        .toString()
                )
                val request =
                    client
                        .waitFor("request") {
                            it.getString("instance") == secondInstance &&
                                it.optJSONObject("model") != null
                        }
                        .getJSONObject("model")
                val allow =
                    request.getJSONArray("actions").objects().first {
                        it.getString("id") == "approve"
                    }
                client.send(
                    JSONObject()
                        .put("local", "request")
                        .put("instance", secondInstance)
                        .put("request", request.getString("id"))
                        .put("generation", request.getLong("generation"))
                        .put("action", allow.getString("id"))
                        .put("fields", JSONObject())
                        .toString()
                )
                client.waitFor("view") {
                    it.getString("instance") == secondInstance &&
                        it.toString().contains("android-private-tool") &&
                        settled(it.getJSONObject("view")).isNotEmpty()
                }
                client.send(
                    JSONObject().put("local", "select").put("instance", firstInstance).toString()
                )
                client.waitFor("selected") { it.getString("instance") == firstInstance }
                client.send(
                    JSONObject(Intents.command("login", listOf("provider" to "anthropic")))
                        .put("instance", firstInstance)
                        .toString()
                )
                val credential =
                    client
                        .waitFor("request") {
                            it.getString("instance") == firstInstance &&
                                it.optJSONObject("model")
                                    ?.optJSONObject("input")
                                    ?.optBoolean("secret") == true
                        }
                        .getJSONObject("model")
                client.send(
                    JSONObject()
                        .put("local", "request")
                        .put("instance", firstInstance)
                        .put("request", credential.getString("id"))
                        .put("generation", credential.getLong("generation"))
                        .put("action", "cancel")
                        .put("fields", JSONObject())
                        .toString()
                )
                client.waitFor("request") {
                    it.getString("instance") == firstInstance && it.optJSONObject("model") == null
                }
            }
        } finally {
            directory.deleteRecursively()
        }
    }
}
