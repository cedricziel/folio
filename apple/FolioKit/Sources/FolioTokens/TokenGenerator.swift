import Foundation

/// Turns `design/tokens.json` into the SwiftUI theme and the named-color asset catalog.
public enum TokenGenerator {
    public enum Failure: Error, Equatable, CustomStringConvertible {
        case invalidHex(String)
        case unknownAlias(token: String, target: String)
        case unmappedTextStyle(String)

        public var description: String {
            switch self {
            case let .invalidHex(hex): "Invalid hex color '\(hex)'"
            case let .unknownAlias(token, target): "Token '\(token)' aliases unknown '\(target)'"
            case let .unmappedTextStyle(name): "No Dynamic Type mapping for text style '\(name)'"
            }
        }
    }

    public struct ColorToken: Equatable {
        public enum Value: Equatable {
            case themed(light: String, dark: String)
            case alias(String)
        }

        public let name: String
        public let value: Value
    }

    public struct TextStyle: Equatable {
        public let name: String
        public let weight: Int
    }

    public struct Dimension: Equatable {
        public let name: String
        public let points: Int
    }

    public struct Tokens: Equatable {
        public let colors: [ColorToken]
        public let textStyles: [TextStyle]
        public let spacing: [Dimension]
        public let radius: [Dimension]
    }

    public struct Output: Equatable {
        public let theme: String
        /// Asset catalog files keyed by path relative to the resources directory.
        public let files: [String: String]
    }

    /// Dynamic Type mapping from design/README.md ("Type"). Monospaced styles carry tabular digits.
    private static let textStyleMap: [String: (style: String, monospaced: Bool)] = [
        "display": ("largeTitle", false),
        "title": ("title2", false),
        "heading": ("headline", false),
        "body": ("body", false),
        "body-sm": ("subheadline", false),
        "label": ("caption", false),
        "meta": ("caption", true),
        "ocr": ("callout", true),
    ]

    private static let weightNames: [Int: String] = [
        300: "light", 400: "regular", 500: "medium", 600: "semibold", 700: "bold",
    ]

    public static func camelCase(_ name: String) -> String {
        let parts = name.split(separator: "-")
        guard let first = parts.first else { return name }
        return String(first) + parts.dropFirst().map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined()
    }

    public static func parse(_ data: Data) throws -> Tokens {
        let raw = try JSONDecoder().decode(RawTokens.self, from: data)
        let colors = raw.color.tokens.map { token in
            switch token.value {
            case let .themed(light, dark):
                ColorToken(name: token.name, value: .themed(light: light, dark: dark))
            case let .alias(reference):
                ColorToken(
                    name: token.name,
                    value: .alias(reference.trimmingCharacters(in: CharacterSet(charactersIn: "{}")))
                )
            }
        }
        return Tokens(
            colors: colors,
            textStyles: raw.type.groups.flatMap(\.styles).map {
                TextStyle(name: $0.name, weight: $0.fontWeight)
            },
            spacing: raw.spacing.tokens.map { Dimension(name: $0.name, points: pixels($0.value)) },
            radius: raw.radius.tokens.map { Dimension(name: $0.name, points: pixels($0.value)) }
        )
    }

    public static func generate(from data: Data) throws -> Output {
        let tokens = try parse(data)
        let known = Set(tokens.colors.map(\.name))
        for color in tokens.colors {
            if case let .alias(target) = color.value, !known.contains(target) {
                throw Failure.unknownAlias(token: color.name, target: target)
            }
        }
        return try Output(theme: renderTheme(tokens), files: renderCatalog(tokens))
    }

    /// Returns "r,g,b" with three decimals, the form used by asset catalog `components`.
    public static func components(hex: String) throws -> String {
        let digits = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard digits.count == 6, let value = UInt32(digits, radix: 16) else {
            throw Failure.invalidHex(hex)
        }
        let channels = [16, 8, 0].map { Double((value >> UInt32($0)) & 0xFF) / 255 }
        return channels.map { String(format: "%.3f", $0) }.joined(separator: ",")
    }

    private static func pixels(_ value: String) -> Int {
        Int(value.replacingOccurrences(of: "px", with: "")) ?? 0
    }

    private static func renderCatalog(_ tokens: Tokens) throws -> [String: String] {
        var files = ["Colors.xcassets/Contents.json": catalogInfo]
        for color in tokens.colors {
            guard case let .themed(light, dark) = color.value else { continue }
            let path = "Colors.xcassets/\(camelCase(color.name)).colorset/Contents.json"
            files[path] = try colorSet(light: light, dark: dark)
        }
        return files
    }

    private static var info: [String: Any] { ["author": "folio-tokengen", "version": 1] }

    private static var catalogInfo: String {
        json(["info": info])
    }

    private static func colorSet(light: String, dark: String) throws -> String {
        func entry(_ hex: String, dark isDark: Bool) throws -> [String: Any] {
            let parts = try components(hex: hex).split(separator: ",").map(String.init)
            var entry: [String: Any] = [
                "idiom": "universal",
                "color": [
                    "color-space": "srgb",
                    "components": [
                        "red": parts[0], "green": parts[1], "blue": parts[2], "alpha": "1.000",
                    ],
                ],
            ]
            if isDark {
                entry["appearances"] = [["appearance": "luminosity", "value": "dark"]]
            }
            return entry
        }
        return try json([
            "colors": [entry(light, dark: false), entry(dark, dark: true)],
            "info": info,
        ])
    }

    private static func json(_ object: [String: Any]) -> String {
        let data = try! JSONSerialization.data(
            withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        )
        return String(decoding: data, as: UTF8.self) + "\n"
    }

    private static func renderTheme(_ tokens: Tokens) throws -> String {
        var out = """
        // Generated by folio-tokengen from design/tokens.json. Do not edit.

        import SwiftUI

        extension Color {

        """
        for color in tokens.colors {
            let name = camelCase(color.name)
            switch color.value {
            case .themed:
                out += "    public static let \(name) = Color(\"\(name)\", bundle: .module)\n"
            case let .alias(target):
                out += "    public static var \(name): Color { \(camelCase(target)) }\n"
            }
        }
        out += "}\n\npublic enum FolioTheme {\n    public enum Typography {\n"
        for style in tokens.textStyles {
            guard let mapping = textStyleMap[style.name] else {
                throw Failure.unmappedTextStyle(style.name)
            }
            var expression = "Font.\(mapping.style)"
            if mapping.monospaced {
                expression += ".monospaced().monospacedDigit()"
            }
            if style.weight != 400 {
                expression += ".weight(.\(weightNames[style.weight] ?? "regular"))"
            }
            out += "        public static let \(camelCase(style.name)) = \(expression)\n"
        }
        out += "    }\n\n    public enum Spacing {\n"
        for token in tokens.spacing {
            out += "        public static let \(camelCase(token.name)): CGFloat = \(token.points)\n"
        }
        out += "    }\n\n    public enum Radius {\n"
        for token in tokens.radius {
            out += "        public static let \(camelCase(token.name)): CGFloat = \(token.points)\n"
        }
        out += "    }\n}\n"
        return out
    }
}

private struct RawTokens: Decodable {
    struct Color: Decodable {
        enum Value: Decodable {
            case themed(light: String, dark: String)
            case alias(String)

            init(from decoder: any Decoder) throws {
                let container = try decoder.singleValueContainer()
                if let alias = try? container.decode(String.self) {
                    self = .alias(alias)
                } else {
                    let themes = try container.decode([String: String].self)
                    guard let light = themes["light"], let dark = themes["dark"] else {
                        throw DecodingError.dataCorruptedError(
                            in: container, debugDescription: "Color needs light and dark values"
                        )
                    }
                    self = .themed(light: light, dark: dark)
                }
            }
        }

        let name: String
        let value: Value
    }

    struct Style: Decodable {
        let name: String
        let fontWeight: Int
    }

    struct Group: Decodable { let styles: [Style] }
    struct Dimension: Decodable {
        let name: String
        let value: String
    }

    struct Colors: Decodable { let tokens: [Color] }
    struct Types: Decodable { let groups: [Group] }
    struct Dimensions: Decodable { let tokens: [Dimension] }

    let color: Colors
    let type: Types
    let spacing: Dimensions
    let radius: Dimensions
}
