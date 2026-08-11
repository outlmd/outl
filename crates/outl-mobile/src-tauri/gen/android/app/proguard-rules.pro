# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# iroh TLS verifier: the Rust side looks these up by name through JNI, so the
# release minifier must not strip or rename them.
-keep class org.rustls.platformverifier.** { *; }
# JNI bootstrap called from MainActivity.onCreate (see android_jni.rs).
-keep class app.outl.mobile_app.NativeSetup { *; }
# Background-sync JNI (see bg_sync.rs): the Rust .so exports symbols derived
# from this exact class + method names (Java_app_outl_mobile_1app_NativeSync_*),
# so R8 renaming either one breaks background sync in release builds only.
-keep class app.outl.mobile_app.NativeSync { *; }

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile