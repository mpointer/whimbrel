// swift-tools-version: 6.0
import PackageDescription

// The Thread design system for the Whimbrel Mac app. No dependency on the app target.
let package = Package(
    name: "WhimbrelUI",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "WhimbrelUI", targets: ["WhimbrelUI"]),
        .executable(name: "WhimbrelUIGallery", targets: ["WhimbrelUIGallery"]),
    ],
    targets: [
        .target(name: "WhimbrelUI"),
        .executableTarget(name: "WhimbrelUIGallery", dependencies: ["WhimbrelUI"]),
        .testTarget(name: "WhimbrelUITests", dependencies: ["WhimbrelUI"]),
    ]
)
