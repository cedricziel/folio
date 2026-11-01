import Foundation

public extension TokenGenerator.Output {
    static let themePath = "Sources/FolioKit/Theme/FolioTheme.swift"
    static let resourcesPath = "Sources/FolioKit/Resources"

    /// Every generated file keyed by path relative to the package root.
    var filesByPackagePath: [String: String] {
        var all = [Self.themePath: theme]
        for (path, content) in files {
            all["\(Self.resourcesPath)/\(path)"] = content
        }
        return all
    }

    func write(toPackageAt root: URL) throws {
        let manager = FileManager.default
        let catalog = root.appending(path: "\(Self.resourcesPath)/Colors.xcassets")
        if manager.fileExists(atPath: catalog.path) {
            try manager.removeItem(at: catalog)
        }
        for (path, content) in filesByPackagePath {
            let url = root.appending(path: path)
            try manager.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true
            )
            try content.write(to: url, atomically: true, encoding: .utf8)
        }
    }
}
