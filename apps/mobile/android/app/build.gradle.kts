import java.util.Properties
import groovy.json.JsonSlurper

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// The one place the version is computed: the repo VERSION file, major*1_000_000 + minor*1_000 + patch.
val repoVersion = file("../../../../VERSION").readText().trim()
val computedVersionCode = repoVersion.split(".").map { it.toInt() }.let { (major, minor, patch) ->
    major * 1_000_000 + minor * 1_000 + patch
}

val secrets = Properties().apply {
    file("../secrets.properties").takeIf { it.exists() }?.reader()?.use { load(it) }
}

val coreDir = file("../../../../core")

fun String.asBuildConfigString() = "\"" + replace("\\", "\\\\").replace("\"", "\\\"") + "\""

// <cargo target_directory>/aarch64-linux-android/<profile>/libwhimbrel_core.a, honouring any
// configured target-dir (the Mac build symlinks core/target elsewhere).
fun coreStaticLib(profile: String): String {
    val json = providers.exec {
        workingDir = coreDir
        commandLine("cargo", "metadata", "--format-version", "1", "--no-deps")
    }.standardOutput.asText.get()
    val targetDir = (JsonSlurper().parseText(json) as Map<*, *>)["target_directory"]
    return "$targetDir/aarch64-linux-android/$profile/libwhimbrel_core.a"
}

// The shared theme files, top level only (invalid/ and the README stay out of the APK), as generated assets.
abstract class CopyBundledThemes : DefaultTask() {
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val themes: ConfigurableFileCollection
    @get:OutputDirectory abstract val outDir: DirectoryProperty

    @TaskAction fun copy() {
        val out = outDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        themes.files.forEach { it.copyTo(out.resolve(it.name)) }
    }
}

val copyBundledThemes = tasks.register<CopyBundledThemes>("copyBundledThemes") {
    themes.from(fileTree("../../../../themes") { include("*.json") })
    outDir.set(layout.buildDirectory.dir("generated/themeAssets"))
}

// Registered the AGP way so merge, lint and the lint models all depend on the copy.
androidComponents {
    onVariants { v -> v.sources.assets?.addGeneratedSourceDirectory(copyBundledThemes, CopyBundledThemes::outDir) }
}

android {
    namespace = "com.mpointer.whimbrel.mobile"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.mpointer.whimbrel.mobile"
        minSdk = 29
        targetSdk = 35
        versionName = repoVersion
        versionCode = project.findProperty("versionCodeOverride")?.toString()?.toInt() ?: computedVersionCode

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk { abiFilters += "arm64-v8a" }

        externalNativeBuild {
            cmake {
                arguments += listOf(
                    "-DANDROID_STL=c++_static",
                    "-DWHIMBREL_CORE_INCLUDE=${coreDir.resolve("ffi/include")}",
                )
            }
        }

        buildConfigField("String", "GOOGLE_WEB_CLIENT_ID", secrets.getProperty("GOOGLE_WEB_CLIENT_ID", "").asBuildConfigString())
        buildConfigField("String", "SYNC_WORKER_BASE_URL", secrets.getProperty("SYNC_WORKER_BASE_URL", "").asBuildConfigString())
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    // Release signing comes from $WHIMBREL_KEYSTORE_DIR/keystore.properties (see keystore.properties.example).
    // Debug builds never read it; the error below fires only when a release APK is actually packaged.
    val keystoreDir = System.getenv("WHIMBREL_KEYSTORE_DIR")
    // CI only: proves the R8-minified release build without the keystore. Never set it when building to ship.
    val unsignedRelease = System.getenv("WHIMBREL_UNSIGNED_RELEASE") == "1"
    val keystoreProps = Properties().apply {
        keystoreDir?.let { file("$it/keystore.properties") }?.takeIf { it.exists() }?.reader()?.use { load(it) }
    }
    signingConfigs {
        create("release") {
            if (keystoreProps.isNotEmpty()) {
                storeFile = file(keystoreProps.getProperty("storeFile"))
                storePassword = keystoreProps.getProperty("storePassword")
                keyAlias = keystoreProps.getProperty("keyAlias")
                keyPassword = keystoreProps.getProperty("keyPassword")
            }
        }
    }
    gradle.taskGraph.whenReady {
        if (hasTask(":app:packageRelease") && keystoreProps.isEmpty() && !unsignedRelease) {
            throw GradleException(
                "Release signing needs WHIMBREL_KEYSTORE_DIR pointing at a directory with keystore.properties " +
                    "(currently ${keystoreDir ?: "unset"}; expected ${keystoreDir ?: "<dir>"}/keystore.properties). " +
                    "Create one with scripts/create-release-keystore.sh <dir>.",
            )
        }
    }

    buildTypes {
        // The debug demo build installs beside the release build on the same phone.
        debug {
            applicationIdSuffix = ".debug"
            externalNativeBuild { cmake { arguments += "-DWHIMBREL_CORE_A=${coreStaticLib("debug")}" } }
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = if (unsignedRelease) null else signingConfigs.getByName("release")
            externalNativeBuild { cmake { arguments += "-DWHIMBREL_CORE_A=${coreStaticLib("release")}" } }
        }
    }

    // The JVM tests read the same files, invalid/ included.
    sourceSets.getByName("test").resources.srcDir("../../../../themes")

    ndkVersion = "27.2.12479018"
    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.22.1"
        }
    }
}


dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.core)
    implementation(libs.androidx.credentials)
    implementation(libs.androidx.credentials.play)
    implementation(libs.googleid)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.work.runtime.ktx)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material3.navigation.suite)
    implementation(libs.androidx.compose.adaptive)
    implementation(libs.androidx.compose.adaptive.layout)
    implementation(libs.androidx.compose.adaptive.navigation)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.runner)
}

// Rust core for arm64-v8a. Same flags as the `android` CI job: no diarize (no ONNX Runtime for
// Android) and +fp16 for gemm-f16. One task per profile so a multi-variant build gets both archives.
fun registerBuildCore(name: String, release: Boolean) = tasks.register<Exec>(name) {
    workingDir = coreDir
    // whisper-rs-sys's build.rs tests `cfg!(target_os = "macos")` (the HOST, not the target), so on
    // a Mac it asks rustc for `ggml-blas`, which an Android ggml never builds. An empty archive
    // satisfies it; nothing references BLAS symbols. Linux hosts (CI) never ask for it.
    val stubDir = layout.buildDirectory.dir("core-stubs").get().asFile
    val llvmAr = android.ndkDirectory.resolve("toolchains/llvm/prebuilt").listFiles()!!.first().resolve("bin/llvm-ar")
    environment("RUSTFLAGS", "-C target-feature=+fp16 -L native=$stubDir")
    doFirst {
        stubDir.mkdirs()
        val stub = stubDir.resolve("libggml-blas.a")
        if (!stub.exists()) exec { commandLine(llvmAr.absolutePath, "rc", stub.absolutePath) }
    }
    environment("ANDROID_NDK_HOME", android.ndkDirectory.absolutePath)
    // CMake's Android platform module (used by whisper-rs-sys) reads ANDROID_NDK, not the cargo-ndk name.
    environment("ANDROID_NDK", android.ndkDirectory.absolutePath)
    // whisper-rs-sys runs CMake for the Android target; the NDK toolchain file needs Ninja, which
    // the SDK's CMake ships (same one AGP uses). Gradle's PATH has no make for that toolchain.
    val sdkCmakeBin = android.sdkDirectory.resolve("cmake/3.22.1/bin")
    environment("PATH", "$sdkCmakeBin${File.pathSeparator}${System.getenv("PATH")}")
    environment("CMAKE_GENERATOR", "Ninja")
    commandLine(
        listOf("cargo", "ndk", "-t", "arm64-v8a", "build", "-p", "ffi", "--no-default-features") +
            if (release) listOf("--release") else emptyList(),
    )
}
val buildCoreDebug = registerBuildCore("buildCoreDebug", release = false)
val buildCoreRelease = registerBuildCore("buildCoreRelease", release = true)

tasks.configureEach {
    if (name.startsWith("externalNativeBuild") || name.startsWith("configureCMake")) {
        // AGP names the release variant's CMake tasks "RelWithDebInfo" (buildCMakeRelWithDebInfo[arm64-v8a]).
        dependsOn(if (name.contains("Release") || name.contains("RelWithDebInfo")) buildCoreRelease else buildCoreDebug)
    }
}
