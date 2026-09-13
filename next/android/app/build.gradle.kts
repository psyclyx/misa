plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Native artifacts are supplied by the Android Nix package, one directory per ABI.
val misaAbis = (project.findProperty("misaAbis") as String? ?: "x86_64,arm64-v8a,armeabi-v7a")
    .split(",").map { it.trim() }.filter { it.isNotEmpty() }

android {
    namespace = "org.misa.app"
    compileSdk = 37
    buildToolsVersion = "37.0.0"
    ndkVersion = "29.0.14206865"
    defaultConfig {
        applicationId = "org.misa.app"
        testInstrumentationRunner = "org.misa.app.ClientInstrumentation"
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
