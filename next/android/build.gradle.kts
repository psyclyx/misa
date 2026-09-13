plugins {
    id("com.android.application") version "9.1.1" apply false
    // Select KGP for AGP's built-in Kotlin; do not apply the legacy Android plugin.
    id("org.jetbrains.kotlin.android") version "2.3.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.3.21" apply false
}
