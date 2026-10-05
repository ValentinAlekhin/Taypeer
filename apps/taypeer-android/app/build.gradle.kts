plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "dev.taypeer"
    compileSdk = 35
    buildToolsVersion = "35.0.0"
    ndkVersion = "27.2.12479018"
    defaultConfig {
        applicationId = "dev.taypeer"
        minSdk = 31
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0-dev"
        ndk { abiFilters += "arm64-v8a" }
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    val releaseStore = providers.environmentVariable("TAYPEER_ANDROID_KEYSTORE").orNull
    if (releaseStore != null) {
        signingConfigs.create("externalRelease") {
            storeFile = file(releaseStore)
            check(!storeFile!!.canonicalPath.startsWith(rootDir.parentFile.parentFile.canonicalPath + "/")) {
                "Keep the release keystore outside the repository"
            }
            storePassword = providers.environmentVariable("TAYPEER_ANDROID_STORE_PASSWORD").orNull
            keyAlias = providers.environmentVariable("TAYPEER_ANDROID_KEY_ALIAS").orNull
            keyPassword = providers.environmentVariable("TAYPEER_ANDROID_KEY_PASSWORD").orNull
        }
        buildTypes.getByName("release").signingConfig = signingConfigs.getByName("externalRelease")
    }
    buildFeatures { compose = true; aidl = true }
    bundle { language { enableSplit = false } }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    sourceSets["androidTest"].assets.srcDir("../../../tests/fixtures/dev6")
    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("generated/jniLibs"))
    sourceSets["main"].java.srcDir(layout.buildDirectory.dir("generated/uniffi"))
    packaging { resources.excludes += "/META-INF/{AL2.0,LGPL2.1}" }
}

val nativeBuild = tasks.register<Exec>("nativeBuild") {
    workingDir(rootDir)
    commandLine("sh", "build-native.sh")
    // Cargo tracks transitive modules, include_str resources, features and toolchain flags.
    // Do not maintain a second incomplete dependency graph that can package stale bindings.
    outputs.upToDateWhen { false }
    outputs.dir(layout.buildDirectory.dir("generated/uniffi"))
    outputs.dir(layout.buildDirectory.dir("generated/jniLibs"))
}
tasks.named("preBuild").configure { dependsOn(nativeBuild) }

dependencyLocking { lockAllConfigurations() }

dependencies {
    implementation(platform("androidx.compose:compose-bom:2025.04.01"))
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.1")
    debugImplementation("androidx.compose.ui:ui-tooling")
    androidTestImplementation(platform("androidx.compose:compose-bom:2025.04.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
