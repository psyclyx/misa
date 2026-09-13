package org.misa.app

import android.app.Activity
import android.app.Instrumentation
import android.content.ContentValues
import android.os.Bundle
import android.provider.MediaStore
import android.graphics.Bitmap
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

/** Runs the actual JNI/iroh/file boundary in an emulator; no test-only transport. */
class ClientInstrumentation : Instrumentation() {
    private var options = Bundle()
    override fun onCreate(arguments: Bundle?) { super.onCreate(arguments); options = arguments ?: Bundle(); start() }
    override fun onStart() {
        val result = Bundle()
        try {
            val ticket = requireNotNull(options.getString("ticket")) { "provide -e ticket <daemon ticket>" }
            incrementalWork()
            exercise(ticket)
            result.putString("stream", "\nPASS: 2 incremental work checks; persistent reconnect, blob upload/fetch, directed save destination\n")
            result.putInt("tests", 5)
            finish(Activity.RESULT_OK, result)
        } catch (error: Throwable) {
            result.putString("stream", "\nFAIL: ${error.stackTraceToString()}\n")
            finish(Activity.RESULT_CANCELED, result)
        }
    }

    private inner class Connection(ticket: String, directory: File) : AutoCloseable {
        val events = LinkedBlockingQueue<JSONObject>()
        private val tree = ViewTree()
        val handle = Native.connect(ticket, directory.absolutePath, Listener {
            val event = JSONObject(it)
            when (event.optString("kind")) {
                "view" -> { tree.reset(Wire.parseNode(event.getJSONObject("view"))); events.offer(event) }
                "changes" -> events.offer(JSONObject().put("kind", "view").put("view", testJson(tree.apply(event.getJSONArray("changes")))))
                else -> events.offer(event)
            }
        })
        init { check(handle != 0L) }
        fun send(json: String) { check(Native.send(handle, json)) { "connection refused local input" } }
        fun waitFor(kind: String, accept: (JSONObject) -> Boolean = { true }): JSONObject {
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(45)
            while (System.nanoTime() < deadline) {
                val event = events.poll(1, TimeUnit.SECONDS) ?: continue
                check(event.optString("kind") != "fault") { event.toString() }
                if (event.optString("kind") == kind && accept(event)) return event
            }
            error("timed out waiting for $kind")
        }
        override fun close() { Native.disconnect(handle) }
    }
    private fun nodes(root: JSONObject): List<JSONObject> = buildList {
        add(root)
        val children = root.optJSONArray("children") ?: JSONArray()
        for (i in 0 until children.length()) addAll(nodes(children.getJSONObject(i)))
    }
    private fun settled(view: JSONObject): Set<String> = nodes(view).filter {
        it.optString("role") == "message.assistant" && it.optString("state") in listOf("done", "failed", "cancelled")
    }.map { it.optString("id") }.toSet()
    private fun Connection.turn(text: String, before: JSONObject, attachments: List<JSONObject> = emptyList()): JSONObject {
        val previous = settled(before)
        send(Intents.prompt(text, attachments))
        return waitFor("view") { event ->
            val view = event.getJSONObject("view")
            view.toString().contains(text) && settled(view).any { it !in previous } && !view.toString().contains("turn.cancel")
        }.getJSONObject("view")
    }

    // The test harness materializes JSON for assertions; production rendering keeps Node references.
    private fun testJson(node: Node): JSONObject = JSONObject().put("id", node.id).put("role", node.role)
        .put("state", node.state).put("children", JSONArray(node.children.map(::testJson)))
        .put("text", (node.shape as? Shape.Text)?.spans?.joinToString("") { it.text } ?: "")
        .put("actions", JSONArray(node.actions.map { JSONObject().put("id", it.id) }))

    private fun incrementalWork() {
        fun tree(size: Int): ViewTree = ViewTree().apply {
            reset(Node("root", "root", null, null, Shape.Section, emptyList(), (0 until size).map {
                Node("msg.$it", "message.user", null, "done", Shape.Text(listOf(Span("old", "plain"))), emptyList(), emptyList())
            }))
        }
        fun replace(tree: ViewTree) {
            tree.apply(JSONArray("""[{"ops":[{"op":"replace","id":"msg.0","node":{"id":"msg.0","role":"message.user","kind":{"shape":"text","spans":[{"text":"new"}]}}}]}]"""))
        }
        val small = tree(1); val large = tree(1000)
        val untouched = large.root!!.children.last()
        replace(small); replace(large)
        check(small.touched == 2 && large.touched == 2)
        check(large.root!!.children.last() === untouched)
        val canonical = large.root
        val streams = LiveStreams()
        streams.apply(JSONObject("""{"update":"current","stream":{"id":"live.text","role":"message.assistant","text":"é"}}"""))
        streams.apply(JSONObject("""{"update":"append","id":"live.text","offset":2,"text":"!"}"""))
        check(streams.values.getValue("live.text").text == "é!")
        check(runCatching { streams.apply(JSONObject("""{"update":"append","id":"live.text","offset":1,"text":"bad"}""")) }.isFailure)
        check(large.root === canonical && large.touched == 2)
    }

    private fun exercise(ticket: String) {
        val root = File(targetContext.filesDir, "instrumentation-client").apply { deleteRecursively(); mkdirs() }
        val peer = File(targetContext.filesDir, "instrumentation-peer").apply { deleteRecursively(); mkdirs() }
        try {
            var saved: JSONObject
            Connection(ticket, root).use { client ->
                val initial = client.waitFor("view").getJSONObject("view")
                saved = client.turn("android-first", initial)
            }
            val identity = File(root, "identity").readBytes()
            val snapshot = root.listFiles()!!.single { it.name.startsWith("view-") }.readText()
            check(snapshot.contains("android-first"))
            check(JSONObject(JSONArray(snapshot).getJSONObject(0).toString()).has("epoch"))
            Connection(ticket, peer).use { other ->
                val initial = other.waitFor("view").getJSONObject("view")
                other.turn("android-while-away", initial)
            }
            Connection(ticket, root).use { client ->
                client.waitFor("sync") { it.optString("mode") == "changes" }
                saved = client.waitFor("view") { it.toString().contains("android-while-away") }.getJSONObject("view")
                check(File(root, "identity").readBytes().contentEquals(identity))
                val pictureSource = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888).apply { eraseColor(android.graphics.Color.RED) }
                val png = ByteArrayOutputStream().use { output -> pictureSource.compress(Bitmap.CompressFormat.PNG, 100, output); output.toByteArray() }
                pictureSource.recycle()
                val upload = File(targetContext.cacheDir, "instrumentation-upload.png").apply { writeBytes(png) }
                client.send(JSONObject().put("local", "upload").put("path", upload.absolutePath).put("name", "picture.png").put("media", "image/png").toString())
                val blob = client.waitFor("uploaded").getJSONObject("blob")
                check(!upload.exists())
                saved = client.turn("android-attachment", saved, listOf(blob))
                check(saved.toString().contains("attachment.save"))
                client.send(JSONObject().put("local", "fetch").put("hash", blob.getString("hash")).toString())
                val fetched = client.waitFor("blob")
                check(File(fetched.getString("path")).readBytes().contentEquals(png))
                val picture = requireNotNull(android.graphics.BitmapFactory.decodeFile(fetched.getString("path")))
                check(picture.width == 1 && picture.height == 1)
                picture.recycle()
                val attachment = nodes(saved).first { node ->
                    val actions = node.optJSONArray("actions") ?: JSONArray()
                    (0 until actions.length()).any { actions.getJSONObject(it).optString("id") == "attachment.save" }
                }
                client.send(JSONObject().put("intent", "action").put("node", attachment.getString("id")).put("action", "attachment.save").toString())
                val download = client.waitFor("download")
                val resolver = targetContext.contentResolver
                val values = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, "misa-instrumentation.png")
                    put(MediaStore.Downloads.MIME_TYPE, "image/png")
                    put(MediaStore.Downloads.RELATIVE_PATH, "Download/Misa-tests")
                }
                val destination = requireNotNull(resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values))
                try {
                    DocumentFiles.save(resolver, File(download.getString("path")), destination)
                    check(resolver.openInputStream(destination)!!.use { it.readBytes() }.contentEquals(png))
                } finally { resolver.delete(destination, null, null) }
            }
            val offline = ticket.substringBefore('@') + "@10.0.2.2:1:" + ticket.substringAfterLast(':')
            Connection(offline, root).use { client ->
                check(client.waitFor("view").toString().contains("android-attachment"))
            }
        } finally { root.deleteRecursively(); peer.deleteRecursively() }
    }
}
