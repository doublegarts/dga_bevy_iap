// CI-only wrapper that builds the android/ module on its own.
pluginManagement {
    repositories { google(); mavenCentral(); gradlePluginPortal() }
}
dependencyResolutionManagement {
    repositories { google(); mavenCentral() }
}
rootProject.name = "dga-bevy-iap-ci"
include(":dga-bevy-iap")
project(":dga-bevy-iap").projectDir = file("../../android")
