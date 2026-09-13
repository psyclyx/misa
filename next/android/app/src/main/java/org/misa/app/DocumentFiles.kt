package org.misa.app

import android.content.ContentResolver
import android.net.Uri
import java.io.File

/** The chosen destination stays entirely on the phone. */
object DocumentFiles {
    fun save(resolver: ContentResolver, source: File, destination: Uri) {
        resolver.openOutputStream(destination, "wt")?.use { output ->
            source.inputStream().use { it.copyTo(output) }
        } ?: error("could not open the chosen destination")
    }
}
