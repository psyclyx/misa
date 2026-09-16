package org.misa.app

/**
 * The JNI seam.
 *
 * Everything above this line is Kotlin and everything below it is the same Rust client the
 * terminal, browser, and pixel frontends link. The seam is JSON in both directions. Rust owns
 * scoped replicas and command correlation; Kotlin owns local pages and applies each captured
 * document transaction together.
 */
object Native {
    init {
        System.loadLibrary("misa_android")
    }

    /** The native client's version, so a bug report can say which one it was. */
    external fun version(): String

    external fun cached(storage: String, hash: String): String?

    /**
     * Open a workspace, optionally connecting a ticket or pairing string. The handle owns several
     * daemon relationships and local session instances.
     */
    external fun connect(ticket: String, storage: String, listener: Listener): Long

    /** Send one intent, as the JSON of [org.misa.app.Intents]. */
    external fun send(handle: Long, intent: String): Boolean

    /** Pull one complete transaction/result after a coalesced native wakeup. */
    external fun poll(handle: Long): String?

    /** Close this workspace's local interests and forget the handle. */
    external fun disconnect(handle: Long)
}

/**
 * Coalesced wakeups; the surface pulls complete events through Native.poll.
 *
 * A plain class with a method named `onEvent` rather than a lambda: the native side calls it by
 * name through JNI, and a Kotlin function type compiles to a class whose method is `invoke`, which
 * is not the name the other end knows.
 */
class Listener(private val onEvent: (String) -> Unit) {
    @Suppress("unused") fun onEvent(json: String) = onEvent.invoke(json)
}
