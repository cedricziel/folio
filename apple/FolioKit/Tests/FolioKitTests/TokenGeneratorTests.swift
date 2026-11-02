import Foundation
import Testing

@testable import FolioTokens

private let sample = Data(
    """
    {
      "color": {"tokens": [
        {"name": "paper-raised", "value": {"light": "#ffffff", "dark": "#1b1f1d"}},
        {"name": "focus", "value": "{paper-raised}"}
      ]},
      "type": {"groups": [{"styles": [
        {"name": "body-sm", "fontSize": "13px", "fontWeight": 400},
        {"name": "meta", "fontSize": "12px", "fontWeight": 400}
      ]}]},
      "spacing": {"tokens": [{"name": "space-4", "value": "16px"}]},
      "radius": {"tokens": [{"name": "radius-lg", "value": "10px"}]}
    }
    """.utf8
)

struct TokenGeneratorTests {
    @Test func camelCasesTokenNames() {
        #expect(TokenGenerator.camelCase("paper-raised") == "paperRaised")
        #expect(TokenGenerator.camelCase("on-archive") == "onArchive")
        #expect(TokenGenerator.camelCase("paper") == "paper")
        #expect(TokenGenerator.camelCase("space-12") == "space12")
    }

    @Test func parsesColorsAndAliases() throws {
        let tokens = try TokenGenerator.parse(sample)
        #expect(tokens.colors.count == 2)
        #expect(tokens.colors[0].value == .themed(light: "#ffffff", dark: "#1b1f1d"))
        #expect(tokens.colors[1].value == .alias("paper-raised"))
    }

    @Test func rejectsAliasToUnknownToken() {
        let json = Data(
            """
            {"color": {"tokens": [{"name": "focus", "value": "{nope}"}]},
             "type": {"groups": []}, "spacing": {"tokens": []}, "radius": {"tokens": []}}
            """.utf8
        )
        #expect(throws: TokenGenerator.Failure.self) {
            _ = try TokenGenerator.generate(from: json)
        }
    }

    @Test func rejectsMalformedHex() {
        #expect(throws: TokenGenerator.Failure.self) {
            _ = try TokenGenerator.components(hex: "#12345")
        }
    }

    @Test func convertsHexToSRGBComponents() throws {
        let white = try TokenGenerator.components(hex: "#ffffff")
        #expect(white == "1.000,1.000,1.000")
        let ink = try TokenGenerator.components(hex: "#1a1f1c")
        #expect(ink == "0.102,0.122,0.110")
    }

    @Test func emitsColorSetWithLightAndDarkAppearances() throws {
        let output = try TokenGenerator.generate(from: sample)
        let colorSet = try #require(
            output.files["Colors.xcassets/paperRaised.colorset/Contents.json"]
        )
        #expect(colorSet.contains("\"value\" : \"dark\""))
        #expect(colorSet.contains("\"red\" : \"1.000\""))
        #expect(output.files["Colors.xcassets/focus.colorset/Contents.json"] == nil)
    }

    @Test func emitsHighContrastColorsForBothAppearances() throws {
        let data = Data(
            """
            {"color": {"tokens": [{"name": "ink", "value": {"light": "#1a1f1c", "dark": "#e8ebe7"},
              "highContrast": {"light": "#000000", "dark": "#ffffff"}}]},
             "type": {"groups": []}, "spacing": {"tokens": []}, "radius": {"tokens": []}}
            """.utf8
        )
        let output = try TokenGenerator.generate(from: data)
        let colorSet = try #require(output.files["Colors.xcassets/ink.colorset/Contents.json"])
        let catalog = try #require(JSONSerialization.jsonObject(with: Data(colorSet.utf8)) as? [String: Any])
        let colors = try #require(catalog["colors"] as? [[String: Any]])
        let light = try #require(
            colors.first {
                $0["appearances"] as? [[String: String]] == [
                    ["appearance": "luminosity", "value": "light"],
                    ["appearance": "contrast", "value": "high"],
                ]
            })
        let dark = try #require(
            colors.first {
                $0["appearances"] as? [[String: String]] == [
                    ["appearance": "luminosity", "value": "dark"],
                    ["appearance": "contrast", "value": "high"],
                ]
            })
        #expect(
            (light["color"] as? [String: Any])?["components"] as? [String: String] == [
                "red": "0.000", "green": "0.000", "blue": "0.000", "alpha": "1.000",
            ])
        #expect(
            (dark["color"] as? [String: Any])?["components"] as? [String: String] == [
                "red": "1.000", "green": "1.000", "blue": "1.000", "alpha": "1.000",
            ])
    }

    @Test func emitsThemeWithAliasSpacingAndRadius() throws {
        let theme = try TokenGenerator.generate(from: sample).theme
        #expect(theme.contains("static let paperRaised = Color(\"paperRaised\", bundle: .module)"))
        #expect(theme.contains("static var focus: Color { paperRaised }"))
        #expect(theme.contains("static let space4: CGFloat = 16"))
        #expect(theme.contains("static let radiusLg: CGFloat = 10"))
        #expect(theme.contains("static let bodySm = Font.subheadline"))
        #expect(theme.contains("static let meta = Font.caption.monospaced().monospacedDigit()"))
    }

    @Test func outputIsDeterministic() throws {
        let first = try TokenGenerator.generate(from: sample)
        let second = try TokenGenerator.generate(from: sample)
        #expect(first.theme == second.theme)
        #expect(first.files == second.files)
    }
}
