// Android library module with the Kotlin bridge for dga_bevy_iap.
// Include it from your app's settings.gradle.kts; see the crate README.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.dga.bevy.iap"
    compileSdk = 35

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    api("com.revenuecat.purchases:purchases:10.26.0")
}
