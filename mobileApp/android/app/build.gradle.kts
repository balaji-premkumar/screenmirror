import java.util.Properties

plugins {
    id("com.android.application")
    id("kotlin-android")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// ── Release signing ─────────────────────────────────────────
// Create android/key.properties (git-ignored) with:
//   storeFile=/absolute/path/to/upload-keystore.jks
//   storePassword=...
//   keyAlias=upload
//   keyPassword=...
// Without it the release build falls back to the debug key and says so —
// debug-signed APKs cannot be published and are trivially re-signed by anyone.
val keystorePropertiesFile = rootProject.file("key.properties")
val keystoreProperties = Properties().apply {
    if (keystorePropertiesFile.exists()) {
        keystorePropertiesFile.inputStream().use { load(it) }
    }
}
val hasReleaseSigning = keystoreProperties.getProperty("storeFile") != null

// ABIs to build the Rust library for. Override for a faster local loop:
//   flutter build apk --release -PmirrorAbis=arm64-v8a
val mirrorAbis: List<String> =
    (project.findProperty("mirrorAbis") as String? ?: "arm64-v8a,armeabi-v7a,x86_64")
        .split(",").map { it.trim() }.filter { it.isNotEmpty() }

android {
    namespace = "com.mirror.stream_mobile_app"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = JavaVersion.VERSION_17.toString()
    }

    defaultConfig {
        // ...
        applicationId = "com.mirror.stream_mobile_app"
        minSdk = 24 // Required for some USB/MediaCodec features
        targetSdk = 34
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }

    tasks.register("buildRust") {
        doLast {
            // Rust target triple and NDK sysroot directory for each ABI.
            val abiTriples = mapOf(
                "arm64-v8a"   to "aarch64-linux-android",
                "armeabi-v7a" to "armv7-linux-androideabi",
                "x86_64"      to "x86_64-linux-android",
                "x86"         to "i686-linux-android"
            )
            // The sysroot directory name is not always the Rust triple.
            val abiSysroots = mapOf(
                "arm64-v8a"   to "aarch64-linux-android",
                "armeabi-v7a" to "arm-linux-androideabi",
                "x86_64"      to "x86_64-linux-android",
                "x86"         to "i686-linux-android"
            )

            val unknown = mirrorAbis.filterNot { abiTriples.containsKey(it) }
            if (unknown.isNotEmpty()) {
                throw GradleException("Unknown ABI(s) in mirrorAbis: $unknown")
            }

            // cargo-ndk lays the .so files out under jniLibs/<abi>/ itself.
            val cargoArgs = mutableListOf("cargo", "ndk")
            mirrorAbis.forEach { cargoArgs += listOf("-t", it) }
            cargoArgs += listOf("-o", file("src/main/jniLibs").absolutePath, "build", "--release")
            exec {
                workingDir("../../rust")
                commandLine(cargoArgs)
            }

            // Ship libc++_shared.so alongside the Rust library — the NDK
            // toolchain links against it dynamically.
            val androidExt = project.extensions.getByName<com.android.build.gradle.AppExtension>("android")
            val ndkDir = androidExt.ndkDirectory.absolutePath
            // The prebuilt directory is named after the *host* (linux-x86_64,
            // darwin-x86_64, windows-x86_64...). Hardcoding linux-x86_64 broke
            // the build on every non-Linux machine, so discover it instead.
            val prebuiltRoot = File("$ndkDir/toolchains/llvm/prebuilt")
            val hostDir = prebuiltRoot.listFiles()?.firstOrNull { it.isDirectory }
                ?: throw GradleException("No NDK prebuilt toolchain found under $prebuiltRoot")

            mirrorAbis.forEach { abi ->
                val sysrootAbi = abiSysroots.getValue(abi)
                val lib = File(hostDir, "sysroot/usr/lib/$sysrootAbi/libc++_shared.so")
                if (!lib.exists()) {
                    throw GradleException("libc++_shared.so not found for $abi at $lib")
                }
                copy {
                    from(lib)
                    into("src/main/jniLibs/$abi")
                }
            }
        }
    }

    project.tasks.whenTaskAdded {
        if (name == "mergeDebugJniLibFolders" || name == "mergeReleaseJniLibFolders") {
            dependsOn("buildRust")
        }
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            } else {
                // Falls back so `flutter run --release` still works locally.
                // A debug-signed APK must never be distributed: the debug key
                // is public, so anyone can re-sign and impersonate the build.
                logger.warn(
                    "WARNING: no android/key.properties — signing the release build " +
                    "with the DEBUG key. Do not distribute this artifact."
                )
                signingConfig = signingConfigs.getByName("debug")
            }
        }
    }
}

flutter {
    source = "../.."
}
