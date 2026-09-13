package org.misa.app

/**
 * The JNI seam.
 *
 * Everything above this line is Kotlin and everything below it is the same Rust
 * client the terminal, browser, and pixel frontends link. The seam is JSON in
 * both directions, because both the view tree and every wire message already are
 * `serde` types and a phone should not grow a reader for the protocol's CBOR.
 */
object Native {
    init {
        System.loadLibrary("misa_android")
    }

    /** The native client's version, so a bug report can say which one it was. */
    external fun version(): String

    external fun cached(storage: String, hash: String): String?

    /**
     * Attach to a daemon. `ticket` is a ticket or a pairing string, and the
     * returned handle addresses the connection for [send] and [disconnect].
     */
    external fun connect(ticket: String, storage: String, listener: Listener): Long

    /** Send one intent, as the JSON of [org.misa.app.Intents]. */
    external fun send(handle: Long, intent: String): Boolean

    /** Close the connection and forget the handle. */
    external fun disconnect(handle: Long)
}

/**
 * Where the native client delivers what the session said.
 *
 * A plain class with a method named `onEvent` rather than a lambda: the native
 * side calls it by name through JNI, and a Kotlin function type compiles to a
 * class whose method is `invoke`, which is not the name the other end knows.
 */
class Listener(private val onEvent: (String) -> Unit) {
    @Suppress("unused")
    fun onEvent(json: String) = onEvent.invoke(json)
}
