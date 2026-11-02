import Foundation
import Testing

@testable import FolioTokens

private let contrastPairs: [(foreground: String, background: String, minimum: Double)] = [
    ("ink", "paper", 7),
    ("ink", "paper-raised", 7),
    ("ink", "paper-sunken", 7),
    ("ink-muted", "paper", 7),
    ("ink-muted", "paper-raised", 7),
    ("ink-muted", "paper-sunken", 7),
    ("archive", "paper", 7),
    ("archive", "paper-raised", 7),
    ("archive", "paper-sunken", 7),
    ("danger", "paper", 7),
    ("danger", "paper-raised", 7),
    ("danger", "paper-sunken", 7),
    ("pending", "paper", 7),
    ("pending", "paper-raised", 7),
    ("pending", "paper-sunken", 7),
    ("archive", "archive-tint", 7),
    ("danger", "danger-tint", 7),
    ("pending", "pending-tint", 7),
    ("on-archive", "archive", 7),
    ("on-archive", "archive-strong", 7),
    ("control-border", "paper", 4.5),
    ("control-border", "paper-raised", 4.5),
    ("control-border", "paper-sunken", 4.5),
    ("ink-faint", "paper-raised", 4.5),
]

struct TokenContrastTests {
    @Test(arguments: ["light", "dark"], contrastPairs)
    func increasedContrastMeetsReadabilityTargets(
        theme: String, pair: (foreground: String, background: String, minimum: Double)
    ) throws {
        let packageRoot = URL(filePath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let tokensURL = packageRoot.appending(path: "../../design/tokens.json")
        let output = try TokenGenerator.generate(from: Data(contentsOf: tokensURL))
        let foreground = try luminance(of: pair.foreground, theme: theme, output: output)
        let background = try luminance(of: pair.background, theme: theme, output: output)
        let ratio = (max(foreground, background) + 0.05) / (min(foreground, background) + 0.05)

        #expect(ratio >= pair.minimum, "\(theme) \(pair.foreground) on \(pair.background): \(ratio):1")
    }

    private func luminance(of name: String, theme: String, output: TokenGenerator.Output) throws -> Double {
        let path = "Colors.xcassets/\(TokenGenerator.camelCase(name)).colorset/Contents.json"
        let data = try Data(#require(output.files[path]).utf8)
        let catalog = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let entries = try #require(catalog["colors"] as? [[String: Any]])
        let entry = try #require(
            entries.first {
                $0["appearances"] as? [[String: String]] == [
                    ["appearance": "luminosity", "value": theme], ["appearance": "contrast", "value": "high"],
                ]
            }
        )
        let color = try #require(entry["color"] as? [String: Any])
        let components = try #require(color["components"] as? [String: String])
        let redComponent = try #require(components["red"])
        let greenComponent = try #require(components["green"])
        let blueComponent = try #require(components["blue"])
        let red = try #require(Double(redComponent))
        let green = try #require(Double(greenComponent))
        let blue = try #require(Double(blueComponent))
        return linear(red) * 0.2126 + linear(green) * 0.7152 + linear(blue) * 0.0722
    }

    private func linear(_ component: Double) -> Double {
        component <= 0.04045 ? component / 12.92 : pow((component + 0.055) / 1.055, 2.4)
    }
}
