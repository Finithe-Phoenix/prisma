// android/app/build.gradle.kts — Prisma launcher app (Fase 3 skeleton).

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "dev.prismaemu.app"
    compileSdk = 35

    val prismaDebugKeystore = rootProject.file("prisma-debug.keystore")
    signingConfigs {
        if (prismaDebugKeystore.exists()) {
            create("prismaDebug") {
                storeFile = prismaDebugKeystore
                storePassword = "android"
                keyAlias = "androiddebugkey"
                keyPassword = "android"
            }
        }
    }

    defaultConfig {
        applicationId = "dev.prismaemu.app"
        minSdk = 29  // Android 10 — required for our W^X / MAP_JIT story
        targetSdk = 35
        versionCode = 1
        versionName = "0.0.1"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk {
            // We translate to ARM64; no 32-bit ARM, no x86 (already
            // covered by the guest emulation we're building).
            abiFilters += listOf("arm64-v8a")
        }
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
            isJniDebuggable = true   // critical for our JIT pages.
            if (prismaDebugKeystore.exists()) {
                signingConfig = signingConfigs.getByName("prismaDebug")
            }
            ndk {
                // Desktop UI smoke tests run on the x86_64 AVD. Production
                // execution remains arm64-v8a, where the Prisma DBT runs.
                abiFilters += "x86_64"
            }
        }
        release {
            isMinifyEnabled = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
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
}

dependencies {
    implementation(libs.core.ktx)
    implementation(libs.lifecycle.runtime.ktx)
    implementation(libs.activity.compose)

    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)

    testImplementation(libs.junit)
}

// Task to automatically connect to the Docker Emulator before installing the debug build
tasks.register<Exec>("connectDockerEmulator") {
    group = "emulator"
    description = "Connects ADB to the Docker Android Emulator."
    commandLine("adb", "connect", "localhost:5555")
    isIgnoreExitValue = true
}

tasks.whenTaskAdded {
    if (name.startsWith("install") && name.endsWith("Debug")) {
        dependsOn("connectDockerEmulator")
    }
}
