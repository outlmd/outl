import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 36
    namespace = "app.outl.mobile_app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "app.outl.mobile_app"
        minSdk = 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    // iroh's TLS verifier (`rustls-platform-verifier`) calls into a Kotlin
    // component (`org.rustls.platformverifier.CertificateVerifier`) that must be
    // bundled in the app; the crate ships it as an .aar (not yet on Maven —
    // rustls/rustls-platform-verifier#115). Vendored under `libs/` from
    // `~/.cargo/.../rustls-platform-verifier-android-0.1.1/maven`. Re-copy if the
    // `rustls-platform-verifier` crate version bumps. See `android_jni.rs`.
    implementation(files("libs/rustls-platform-verifier-0.1.1.aar"))
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    // Background P2P sync (see `OutlBackgroundSync.kt`).
    // `lifecycle-process` supplies `ProcessLifecycleOwner` — the app-level
    // "went to background" signal, whose 700ms debounce is what keeps a screen
    // rotation from being reported as a backgrounding.
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    // `work-runtime`, NOT `work-runtime-ktx`: the -ktx artifact has been empty
    // since 2.9.0 (CoroutineWorker and friends moved into work-runtime) and
    // survives only for compatibility. It brings its own manifest entries —
    // WAKE_LOCK, ACCESS_NETWORK_STATE, RECEIVE_BOOT_COMPLETED,
    // FOREGROUND_SERVICE, plus the androidx.startup initializer — so nothing
    // has to be declared in AndroidManifest.xml for this to work.
    implementation("androidx.work:work-runtime:2.10.5")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")