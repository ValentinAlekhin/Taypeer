plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val product = rootProject.file("../../apps/taypeer-android/app/src/main")
// The separate spike consumes an immutable, hash-bound successful product build.
val productNativeCapture = File(requireNotNull(System.getenv("TAYPEER_PUBLIC_NATIVE_CAPTURE")) {
    "Run check.py with --product-native-capture from a successful covered product build"
})
val pipeLibraries = layout.buildDirectory.dir("private-pipe")
val pipeKotlin = layout.buildDirectory.dir("generated/private-pipe-kotlin")
val nativeKotlin = layout.buildDirectory.dir("generated/private-native-kotlin")
val nativeLibraries = layout.buildDirectory.dir("private-native")
val captureNativeKotlin = tasks.register<Sync>("capturePrivateNativeKotlin") {
    from(File(productNativeCapture, "uniffi"))
    into(nativeKotlin)
}
val captureNativeLibrary = tasks.register<Sync>("capturePrivateNativeLibrary") {
    from(File(productNativeCapture, "jniLibs"))
    into(nativeLibraries)
}
val capturePipeKotlin = tasks.register<Sync>("capturePrivatePipeKotlin") {
    from(File(product, "java/dev/taypeer/platform/BiometricKeyPipe.kt"))
    from(File(product, "java/dev/taypeer/platform/BiometricOsKey.kt"))
    into(pipeKotlin)
}
tasks.matching { it.name.startsWith("compile") && it.name.endsWith("Kotlin") }.configureEach {
    dependsOn(capturePipeKotlin, captureNativeKotlin)
}
val compilePipe = tasks.register<Exec>("compilePrivatePipe") {
    val host = if (System.getProperty("os.name").startsWith("Mac")) "darwin-x86_64" else "linux-x86_64"
    val compiler = File(System.getenv("ANDROID_HOME"), "ndk/27.2.12479018/toolchains/llvm/prebuilt/$host/bin/aarch64-linux-android31-clang++")
    val output = pipeLibraries.get().file("arm64-v8a/libtaypeer_biometric_pipe.so").asFile
    inputs.file(File(product, "cpp/biometric_key_pipe.cpp"))
    outputs.file(output)
    doFirst { output.parentFile.mkdirs() }
    commandLine(compiler, "-std=c++17", "-Wall", "-Wextra", "-Werror", "-fvisibility=hidden", "-fPIC", "-shared", "-static-libstdc++",
        "-Wl,--exclude-libs,ALL",
        "-Wl,-z,max-page-size=16384", "-Wl,-z,common-page-size=16384",
        File(product, "cpp/biometric_key_pipe.cpp"), "-o", output)
}
tasks.matching { it.name == "preBuild" }.configureEach { dependsOn(compilePipe, captureNativeLibrary) }

android {
    namespace = "dev.taypeer.biometricspike"
    compileSdk = 35
    buildToolsVersion = "35.0.0"
    defaultConfig {
        applicationId = "dev.taypeer.biometricspike"
        minSdk = 31
        targetSdk = 35
        versionCode = 1
        versionName = "PUBLIC-ONLY"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += "arm64-v8a" }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    sourceSets["main"].java.srcDir(pipeKotlin)
    sourceSets["main"].java.srcDir(nativeKotlin)
    sourceSets["main"].jniLibs.srcDir(pipeLibraries)
    sourceSets["main"].jniLibs.srcDir(nativeLibraries)
}

dependencyLocking { lockAllConfigurations() }
dependencies {
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("androidx.fragment:fragment-ktx:1.8.6")
    implementation("androidx.biometric:biometric:1.1.0")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test:rules:1.6.1")
}
