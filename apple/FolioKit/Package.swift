// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FolioKit",
    platforms: [.iOS(.v18), .macOS(.v15)],
    products: [
        .library(name: "FolioKit", targets: ["FolioKit"]),
        .executable(name: "folio-tokengen", targets: ["folio-tokengen"]),
    ],
    dependencies: [
        .package(url: "https://github.com/open-telemetry/opentelemetry-swift.git", exact: "2.6.0"),
        .package(url: "https://github.com/open-telemetry/opentelemetry-swift-core.git", exact: "2.6.0"),
        .package(url: "https://github.com/apple/swift-openapi-generator", from: "1.0.0"),
        .package(url: "https://github.com/apple/swift-openapi-runtime", from: "1.0.0"),
        .package(url: "https://github.com/apple/swift-openapi-urlsession", from: "1.0.0"),
    ],
    targets: [
        .target(
            name: "FolioAPI",
            dependencies: [
                .product(name: "OpenAPIRuntime", package: "swift-openapi-runtime")
            ],
            plugins: [
                .plugin(name: "OpenAPIGenerator", package: "swift-openapi-generator")
            ]
        ),
        .target(
            name: "FolioKit",
            dependencies: [
                "FolioAPI",
                .product(name: "OpenTelemetryApi", package: "opentelemetry-swift-core"),
                .product(name: "OpenTelemetrySdk", package: "opentelemetry-swift-core"),
                .product(name: "OpenTelemetryConcurrency", package: "opentelemetry-swift-core"),
                .product(name: "OpenTelemetryProtocolExporterHTTP", package: "opentelemetry-swift"),
                .product(name: "URLSessionInstrumentation", package: "opentelemetry-swift"),
                .product(name: "OpenAPIRuntime", package: "swift-openapi-runtime"),
                .product(name: "OpenAPIURLSession", package: "swift-openapi-urlsession"),
            ],
            resources: [.process("Resources")]
        ),
        .target(name: "FolioTokens"),
        .executableTarget(name: "folio-tokengen", dependencies: ["FolioTokens"]),
        .testTarget(
            name: "FolioKitTests",
            dependencies: ["FolioKit", "FolioTokens"]
        ),
    ]
)
