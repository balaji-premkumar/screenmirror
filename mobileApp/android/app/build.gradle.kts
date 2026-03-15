plugins {
    id("com.android.application")
    id("kotlin-android")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

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
        targetSdk = flutter.targetSdkVersion
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
            exec {
                workingDir("../../rust")
                commandLine("cargo", "ndk", "-t", "arm64-v8a", "build", "--release")
            }
            copy {
                from("../../rust/target/aarch64-linux-android/release/librust_lib_stream_mobile_app.so")
                into("src/main/jniLibs/arm64-v8a")
            }
            // Add libc++_shared.so for oboe dependency dynamically finding NDK
            val androidExt = project.extensions.getByName<com.android.build.gradle.AppExtension>("android")
            val ndkDir = androidExt.ndkDirectory.absolutePath
            copy {
                from("$ndkDir/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so")
                into("src/main/jniLibs/arm64-v8a")
            }
        }
    }

    project.tasks.whenTaskAdded {
        if (name == "mergeDebugJniLibFolders" || name == "mergeReleaseJniLibFolders") {
            dependsOn("buildRust")
        }
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

flutter {
    source = "../.."
}
