// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "MacCPUMonitorTray",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(
            name: "MacCPUMonitorTray",
            resources: [.process("Resources")]
        )
    ]
)
