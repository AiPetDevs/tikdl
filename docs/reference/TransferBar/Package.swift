// swift-tools-version:5.9
// TransferBar — the file transfer bar of Lab (Файлы), cut out to be reused: an rsync queue with pause / resume /
// cancel and its SwiftUI views (a pill with the text inside, a ring next to the file, a card with ⏸ ▶ ✕).
import PackageDescription

let package = Package(
    name: "TransferBar",
    platforms: [.macOS(.v14)],
    products: [.library(name: "TransferBar", targets: ["TransferBar"])],
    targets: [
        .target(name: "TransferBar"),
        .executableTarget(name: "TransferBarDemo", dependencies: ["TransferBar"]),
        .testTarget(name: "TransferBarTests", dependencies: ["TransferBar"]),
    ]
)
