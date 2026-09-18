plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.kaigedong.manispocket"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.kaigedong.manispocket"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
    }

    signingConfigs {
        create("dev") {
            storeFile = file("../debug.keystore")
            storePassword = "android"
            keyAlias = "manispocket"
            keyPassword = "android"
        }
    }

    buildTypes {
        debug {
            signingConfig = signingConfigs.getByName("dev")
        }
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("dev")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
    }

    composeOptions {
        kotlinCompilerExtensionVersion = "1.5.15"
    }

    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
}

dependencies {
    // Compose
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")

    // AndroidX
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.activity:activity-compose:1.9.3")

    // JNI for Rust FFI
    implementation("net.java.dev.jna:jna:5.16.0@aar")

    debugImplementation("androidx.compose.ui:ui-tooling")
}

// Build Rust for Android using cargo-ndk.
// Requires: cargo install cargo-ndk, rustup target add aarch64-linux-android, Android NDK
tasks.register<Exec>("buildRustCoreArm64") {
    workingDir = file("../..")
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "-o", "android/app/src/main/jniLibs",
        "build", "--release",
        "--package", "manis-pocket-core",
    )
}

tasks.register<Exec>("generateKotlinBindings") {
    dependsOn("buildRustCoreArm64")
    workingDir = file("../..")
    // Read .so from jniLibs (cargo ndk -o copies it there)
    val libPath = file("src/main/jniLibs/arm64-v8a/libmanis_pocket_core.so").absolutePath
    val outDir = file("src/main/java").absolutePath
    commandLine(
        "cargo", "run", "--release",
        "--bin", "uniffi-bindgen",
        "--package", "manis-pocket-core",
        "generate",
        "--library", libPath,
        "--language", "kotlin",
        "--out-dir", outDir,
    )
}

tasks.register("fixUniffiCode") {
    dependsOn("generateKotlinBindings")
    doLast {
        val f = file("src/main/java/com/kaigedong/manispocket/manis_pocket_core.kt")
        var text = f.readText()
        // CoreError subclasses have 'val `message`' that conflicts with Throwable.message
        text = text.replace("val `message`: kotlin.String", "`message`: kotlin.String")
        f.writeText(text)
    }
}

tasks.named("preBuild") {
    dependsOn("fixUniffiCode")
}
