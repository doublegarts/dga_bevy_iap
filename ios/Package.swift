// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "DgaBevyIap",
    platforms: [.iOS(.v15)],
    products: [
        .library(name: "DgaBevyIap", targets: ["DgaBevyIap"]),
    ],
    dependencies: [
        .package(url: "https://github.com/RevenueCat/purchases-ios-spm.git", from: "5.0.0"),
    ],
    targets: [
        .target(
            name: "DgaBevyIap",
            dependencies: [.product(name: "RevenueCat", package: "purchases-ios-spm")]
        ),
    ]
)
