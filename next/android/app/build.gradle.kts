plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

/**
 * Which ABIs to build the native client for.
 *
 * The default is the one an emulator on this machine runs; a real phone wants
 * `arm64-v8a`. Both are a flag rather than a rewrite because the seam is JNI and
 * the library is architecture-independent above it:
 *
 *     ./gradlew :app:assembleDebug -PmisaAbis=arm64-v8a
 */
val misaAbis: List<String> =
    (project.findProperty("misaAbis") as String? ?: "x86_64")
        .split(",")
        .map { it.trim() }
        .filter { it.isNotEmpty() }

val abiOf =
    mapOf(
        "x86_64" to "x86_64-linux-android",
        "arm64-v8a" to "aarch64-linux-android",
        "armeabi-v7a" to "armv7-linux-androideabi",
    )

android {
    namespace = "org.misa.app"
    compileSdk = 37
    buildToolsVersion = "37.0.0"
    ndkVersion = "29.0.14206865"
    defaultConfig {
        applicationId = "org.misa.app"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters.addAll(misaAbis) }
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging { jniLibs { useLegacyPackaging = true } }
}

/**
 * The native client, built by the NDK before the APK is packaged.
 *
 * The library is a build product and never a checked-in blob: it is the same
 * source the daemon's own tests build, linked into a phone. Gradle runs the
 * shell script rather than reimplementing it, because the exact compiler and
 * sysroot a cross build needs is the script's business and one place to keep it.
 */
val nativeBuildTasks =
    misaAbis.map { abi ->
        val target = abiOf[abi] ?: error("misaAbis names `$abi`, which is not an Android ABI")
        val taskName = "buildMisaNative" + abi.split("-").joinToString("") { it.replaceFirstChar(Char::uppercase) }
        tasks.register(taskName) {
            group = "misa"
            description = "Build libmisa_android.so for $abi with the Android NDK"
            val nativeDir = rootProject.layout.projectDirectory.dir("native").asFile
            val destination = layout.projectDirectory.dir("src/main/jniLibs/$abi").asFile
            inputs.dir(nativeDir.resolve("src"))
            inputs.file(nativeDir.resolve("Cargo.toml"))
            outputs.file(destination.resolve("libmisa_android.so"))
            doLast {
                val ndkVersion = android.ndkVersion
                val environment = mutableMapOf<String, String>()
                val ndkHome =
                    System.getenv("ANDROID_NDK_HOME")
                        ?: System.getenv("ANDROID_HOME")?.let { "$it/ndk/$ndkVersion" }
                        ?: error("set ANDROID_NDK_HOME, or ANDROID_HOME so the NDK can be found")
                environment["ANDROID_NDK_HOME"] = ndkHome
                environment["ANDROID_NATIVE_PROFILE"] = System.getenv("ANDROID_NATIVE_PROFILE") ?: "debug"
                System.getenv("ANDROID_RUST_SOURCE")?.let { environment["ANDROID_RUST_SOURCE"] = it }
                System.getenv("ANDROID_NATIVE_API")?.let { environment["ANDROID_NATIVE_API"] = it }
                val process =
                    ProcessBuilder(nativeDir.resolve("build.sh").absolutePath, target)
                        .directory(nativeDir)
                        .redirectErrorStream(true)
                        .also { it.environment().putAll(environment) }
                        .start()
                val log = process.inputStream.readBytes().decodeToString()
                if (process.waitFor() != 0) {
                    error("the native build for $abi failed:\n$log")
                }
                val profile = environment.getValue("ANDROID_NATIVE_PROFILE")
                val built = nativeDir.resolve("target/$target/$profile/libmisa_android.so")
                destination.mkdirs()
                built.copyTo(destination.resolve("libmisa_android.so"), overwrite = true)
                logger.lifecycle("misa-native: $abi -> ${built.length()} bytes")
            }
        }
    }

tasks.named("preBuild") { dependsOn(nativeBuildTasks) }

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.08.00"))
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")
    // A camera and a decoder, because pairing is a QR code the daemon shows and
    // asking somebody to retype a ticket would be the wrong half of the feature.
    implementation("androidx.camera:camera-core:1.5.0")
    implementation("androidx.camera:camera-camera2:1.5.0")
    implementation("androidx.camera:camera-lifecycle:1.5.0")
    implementation("androidx.camera:camera-view:1.5.0")
    implementation("com.google.zxing:core:3.5.3")
    debugImplementation("androidx.compose.ui:ui-tooling")
    testImplementation("junit:junit:4.13.2")
}
