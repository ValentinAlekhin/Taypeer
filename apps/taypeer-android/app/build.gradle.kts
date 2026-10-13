import org.gradle.testing.jacoco.tasks.JacocoReport
import com.android.build.gradle.internal.tasks.JacocoTask
import org.gradle.kotlin.dsl.withGroovyBuilder
import java.io.File

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    jacoco
}

val qaCoverageEnabled = providers.gradleProperty("qaCoverage").map(String::toBoolean).orElse(false)
val qaJacocoVersion = "0.8.13"
val eagerCorpusAssets = layout.buildDirectory.dir("generated/public-eager-assets")
val captureEagerCorpus = tasks.register<Sync>("capturePublicEagerCorpus") {
    from(file("../../../tests/fixtures/android-eager-icons/seventy-image-entries.taypeer"))
    into(eagerCorpusAssets)
}
// AGP lint reads generated AndroidTest assets outside the asset merge task.
tasks.matching {
    it.name.endsWith("AndroidTestAssets") ||
        (it.name.startsWith("lintAnalyze") && it.name.endsWith("AndroidTest")) ||
        (it.name.startsWith("generate") && it.name.endsWith("AndroidTestLintModel"))
}.configureEach { dependsOn(captureEagerCorpus) }
val biometricPipeLibraries = layout.buildDirectory.dir("private-biometric-pipe")
val compileBiometricPipe = tasks.register<Exec>("compilePrivateBiometricPipe") {
    val sdk = providers.environmentVariable("ANDROID_HOME").orElse(providers.environmentVariable("ANDROID_SDK_ROOT"))
    val host = if (System.getProperty("os.name").startsWith("Mac")) "darwin-x86_64" else "linux-x86_64"
    val source = file("src/main/cpp/biometric_key_pipe.cpp")
    val output = biometricPipeLibraries.get().file("arm64-v8a/libtaypeer_biometric_pipe.so").asFile
    inputs.file(source)
    inputs.property("ndkVersion", "27.2.12479018")
    inputs.property("privatePipeAbi", "arm64-api31-static-hidden-16k-v1")
    outputs.file(output)
    doFirst {
        val compiler = File(sdk.get(), "ndk/27.2.12479018/toolchains/llvm/prebuilt/$host/bin/aarch64-linux-android31-clang++")
        output.parentFile.mkdirs()
        commandLine(compiler, "-std=c++17", "-Wall", "-Wextra", "-Werror", "-fvisibility=hidden", "-fPIC", "-shared", "-static-libstdc++",
            "-Wl,--exclude-libs,ALL", "-Wl,-z,max-page-size=16384", "-Wl,-z,common-page-size=16384", source, "-o", output)
    }
}
tasks.matching { it.name == "preBuild" }.configureEach { dependsOn(compileBiometricPipe) }
// AGP configures the shared jacocoAnt convention independently. This report uses
// its own pinned tools so instrumentation and analysis always share a version.
val qaJacocoReportTools = configurations.create("qaJacocoReportTools") {
    isCanBeConsumed = false
    isCanBeResolved = true
}

android {
    namespace = "dev.taypeer"
    compileSdk = 35
    buildToolsVersion = "35.0.0"
    ndkVersion = "27.2.12479018"
    buildTypes.getByName("debug") {
        enableUnitTestCoverage = qaCoverageEnabled.get()
        enableAndroidTestCoverage = qaCoverageEnabled.get()
    }
    testCoverage { jacocoVersion = qaJacocoVersion }
    defaultConfig {
        applicationId = "dev.taypeer"
        minSdk = 31
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0-dev"
        ndk { abiFilters += "arm64-v8a" }
        testInstrumentationRunner = "dev.taypeer.TaypeerTestRunner"
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
    sourceSets["androidTest"].assets.srcDir("../../../tests/fixtures/dev7")
    sourceSets["androidTest"].assets.srcDir(captureEagerCorpus)
    sourceSets["main"].jniLibs.srcDir(biometricPipeLibraries)
    for (variant in listOf("debug", "release")) {
        sourceSets[variant].jniLibs.srcDir(layout.buildDirectory.dir("generated/jniLibs/$variant"))
        sourceSets[variant].java.srcDir(layout.buildDirectory.dir("generated/uniffi/$variant"))
    }
    packaging { resources.excludes += "/META-INF/{AL2.0,LGPL2.1}" }
}

jacoco { toolVersion = qaJacocoVersion }

fun verifyProductCoverage(originalDirectories: Collection<File>, destination: File, phase: String) {
    val rawRoots = originalDirectories.filter { it.isDirectory }.sortedBy { it.path }
    check(rawRoots.isNotEmpty()) { "No original product classes for coverage instrumentation" }
    val command = listOf("python3", rootProject.file("verify_coverage.py").absolutePath,
        "--output", destination.absolutePath, "--phase", phase) +
        rawRoots.flatMap { listOf("--raw", it.absolutePath) }
    project.exec { commandLine(command) }
}

fun verifyProductCoverage(task: JacocoTask, phase: String) =
    verifyProductCoverage(task.classesDir.files, task.outputForDirs.get().asFile, phase)

tasks.withType<JacocoTask>().configureEach {
    if (name == "jacocoDebug" && qaCoverageEnabled.get()) {
        inputs.property("qaProductRInstrumentation", "agp-8.9.2-jacoco-0.8.13-v1")
        inputs.files(qaJacocoReportTools).withPropertyName("qaProductRInstrumentationTools")
        inputs.file(rootProject.file("verify_coverage.py"))
            .withPropertyName("qaProductRInstrumentationGuard")
        doLast {
            check(com.android.Version.ANDROID_GRADLE_PLUGIN_VERSION == "8.9.2" && qaJacocoVersion == "0.8.13") {
                "Review the product R instrumentation workaround before changing AGP or JaCoCo"
            }
            val destination = outputForDirs.get().asFile.canonicalFile
            check(destination.toPath().startsWith(layout.buildDirectory.get().asFile.canonicalFile.toPath())) {
                "Coverage instrumentation must stay inside the existing JacocoTask output"
            }
            verifyProductCoverage(this as JacocoTask, "before")
            // AGP's generated-R COPY regex also matches Restore/Received/Relay.
            // Repair only these original product classes within this task's output.
            ant.withGroovyBuilder {
                "taskdef"("name" to "qaJacocoInstrument", "classname" to "org.jacoco.ant.InstrumentTask",
                    "classpath" to qaJacocoReportTools.asPath)
                "qaJacocoInstrument"("destdir" to destination) {
                    classesDir.files.filter { it.isDirectory }.sortedBy { it.path }.forEach { raw ->
                        "fileset"("dir" to raw) {
                            "include"("name" to "dev/taypeer/**/R*.class")
                            "exclude"("name" to "**/R.class")
                            "exclude"("name" to "**/R$*.class")
                            "exclude"("name" to "dev/taypeer/bridge/**")
                        }
                    }
                }
            }
            verifyProductCoverage(this as JacocoTask, "after")
        }
    }
}

tasks.register<JacocoReport>("qaCoverageReport") {
    group = "verification"
    description = "Report product Kotlin/Java coverage from this QA run and current unit tests"
    jacocoClasspath = qaJacocoReportTools
    val excluded = listOf(
        "**/R.class", "**/R$*.class", "**/BuildConfig.class",
        "dev/taypeer/bridge/**", "dev/taypeer/platform/I*.class",
        "dev/taypeer/platform/LifecycleProbeService*.class",
        "dev/taypeer/platform/DescriptorProbeService*.class",
        "dev/taypeer/platform/HostDeathProbeService*.class",
        "dev/taypeer/platform/PublicSelectedProvider*.class",
    )
    classDirectories.from(
        fileTree(layout.buildDirectory.dir("tmp/kotlin-classes/debug")) { exclude(excluded) },
        fileTree(layout.buildDirectory.dir("intermediates/javac/debug/compileDebugJavaWithJavac/classes")) { exclude(excluded) },
    )
    sourceDirectories.from(providers.gradleProperty("qaCoverageSources").orElse("src/main/java"))
    executionData.from(fileTree(layout.buildDirectory) {
        include("outputs/unit_test_code_coverage/debugUnitTest/*.exec", "jacoco/testDebugUnitTest.exec")
    })
    providers.gradleProperty("qaCoverageData").orNull?.let { directory ->
        executionData.from(fileTree(directory) { include("**/*.ec") })
    }
    reports {
        html.required.set(true)
        xml.required.set(true)
        csv.required.set(false)
        html.outputLocation.set(layout.buildDirectory.dir("reports/qa-coverage/html"))
        xml.outputLocation.set(layout.buildDirectory.file("reports/qa-coverage/coverage.xml"))
    }
    doFirst {
        check(qaCoverageEnabled.get()) { "Build and run tests with -PqaCoverage=true first" }
        check(executionData.files.any { it.isFile && it.length() > 0 }) { "No current-run JaCoCo execution data" }
        check(classDirectories.files.any { it.isFile }) { "No product debug classes to report" }
        // Report generation must also reject stale or incomplete instrumentation
        // when the existing JacocoTask was up-to-date for this current-run APK.
        // A report-only invocation reads the APK's existing artifacts. AGP's
        // classesDir provider requires producer tasks in this invocation and must
        // not be queried here or rebuild classes after execution data was captured.
        verifyProductCoverage(listOf(
            layout.buildDirectory.dir("tmp/kotlin-classes/debug").get().asFile,
            layout.buildDirectory.dir("intermediates/javac/debug/compileDebugJavaWithJavac/classes").get().asFile,
        ), layout.buildDirectory.dir("intermediates/classes/debug/jacocoDebug/dirs").get().asFile, "after")
    }
}

for (variant in listOf("debug", "release")) {
    val capitalized = variant.replaceFirstChar { it.uppercase() }
    val nativeBuild = tasks.register<Exec>("nativeBuild$capitalized") {
        workingDir(rootDir)
        commandLine("sh", "build-native.sh", variant)
        // Cargo owns dependency tracking; keep debug test capabilities out of release outputs.
        outputs.upToDateWhen { false }
        outputs.dir(layout.buildDirectory.dir("generated/uniffi/$variant"))
        outputs.dir(layout.buildDirectory.dir("generated/jniLibs/$variant"))
    }
    tasks.matching { it.name == "pre${capitalized}Build" }.configureEach { dependsOn(nativeBuild) }
}

dependencyLocking { lockAllConfigurations() }

dependencies {
    add(qaJacocoReportTools.name, "org.jacoco:org.jacoco.ant:$qaJacocoVersion") {
        version { strictly(qaJacocoVersion) }
    }
    implementation(platform("androidx.compose:compose-bom:2025.04.01"))
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.biometric:biometric:1.1.0")
    implementation("androidx.fragment:fragment-ktx:1.8.6")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.1")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("com.google.zxing:core:3.5.3")
    debugImplementation("androidx.compose.ui:ui-tooling")
    androidTestImplementation(platform("androidx.compose:compose-bom:2025.04.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.1")
    androidTestImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
