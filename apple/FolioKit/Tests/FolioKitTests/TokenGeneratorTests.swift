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
