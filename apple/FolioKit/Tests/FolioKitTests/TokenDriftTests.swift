@testable import FolioTokens
import Foundation
import Testing

/// Fails when design/tokens.json changes without regenerating the committed theme and asset catalog.
struct TokenDriftTests {
    private static let packageRoot = URL(filePath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    private static let tokensURL = packageRoot.appending(path: "../../design/tokens.json")

    @Test func committedOutputMatchesGenerator() throws {
        let output = try TokenGenerator.generate(from: Data(contentsOf: Self.tokensURL))
        for (path, expected) in output.filesByPackagePath {
            let url = Self.packageRoot.appending(path: path)
            let committed = try String(contentsOf: url, encoding: .utf8)
            #expect(committed == expected, "\(path) is stale: run `make apple-tokens`")
        }
    }

    @Test func noStaleColorSetsAreCommitted() throws {
        let output = try TokenGenerator.generate(from: Data(contentsOf: Self.tokensURL))
        let catalog = Self.packageRoot.appending(path: "\(TokenGenerator.Output.resourcesPath)/Colors.xcassets")
        let committed = try FileManager.default.contentsOfDirectory(atPath: catalog.path)
            .filter { $0.hasSuffix(".colorset") }
        let expected = output.files.keys.compactMap { $0.split(separator: "/").dropFirst().first }
            .filter { $0.hasSuffix(".colorset") }
        #expect(Set(committed) == Set(expected.map(String.init)))
    }
}
