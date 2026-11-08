import Foundation

/// A Folio server, compared by its canonical base URL: lowercased scheme and host, an internationalised
/// host in its punycode form, IPv6 literals bracketed, no default port, user info, query or fragment, and
/// a case-sensitive path without trailing slashes.
public struct ServerIdentity: Hashable, Sendable, CustomStringConvertible {
    public let scheme: String
    /// Lowercased, ASCII (punycode) host; IPv6 literals without brackets.
    public let host: String
    public let port: Int?
    public let path: String

    public init?(_ url: URL) {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
            let scheme = components.scheme?.lowercased(), scheme == "http" || scheme == "https",
            let rawHost = components.percentEncodedHost, !rawHost.isEmpty,
            let host = Self.canonicalHost(rawHost)
        else { return nil }
        self.scheme = scheme
        self.host = host
        let defaultPort = scheme == "https" ? 443 : 80
        port = components.port.flatMap { $0 == defaultPort ? nil : $0 }
        var path = components.percentEncodedPath
        while path.hasSuffix("/") { path.removeLast() }
        self.path = path
    }

    /// The host as people read it, with a non-default port.
    public var displayHost: String {
        let host = self.host.contains(":") ? "[\(self.host)]" : self.host
        return port.map { "\(host):\($0)" } ?? host
    }

    public var description: String {
        "\(scheme)://\(displayHost)\(path)"
    }

    /// The canonical base URL: no user info, query or fragment.
    public var url: URL? {
        URL(string: description)
    }

    private static func canonicalHost(_ raw: String) -> String? {
        var host = raw
        if host.hasPrefix("[") && host.hasSuffix("]") {
            host = String(host.dropFirst().dropLast())
        }
        if host.contains(":") { return host.lowercased() }
        guard let decoded = host.removingPercentEncoding else { return nil }
        let labels = decoded.precomposedStringWithCanonicalMapping.lowercased().split(
            separator: ".", omittingEmptySubsequences: false)
        var ascii: [String] = []
        for label in labels {
            if label.unicodeScalars.allSatisfy(\.isASCII) {
                ascii.append(String(label))
            } else {
                guard let encoded = Punycode.encode(String(label)) else { return nil }
                ascii.append("xn--" + encoded)
            }
        }
        return ascii.joined(separator: ".")
    }
}

extension ServerIdentity {
    public static func same(_ lhs: URL, _ rhs: URL) -> Bool {
        guard let lhs = ServerIdentity(lhs), let rhs = ServerIdentity(rhs) else { return false }
        return lhs == rhs
    }
}

/// RFC 3492 encoding of one host label.
enum Punycode {
    private static let base = 36, tMin = 1, tMax = 26, skew = 38, damp = 700, initialBias = 72, initialN = 128

    static func encode(_ input: String) -> String? {
        let scalars = input.unicodeScalars.map { Int($0.value) }
        var output = scalars.filter { $0 < 0x80 }.map { Character(UnicodeScalar(UInt8($0))) }
        let basicCount = output.count
        var handled = basicCount
        if basicCount > 0 { output.append("-") }
        var n = initialN, delta = 0, bias = initialBias
        while handled < scalars.count {
            guard let next = scalars.filter({ $0 >= n }).min() else { return nil }
            delta += (next - n) * (handled + 1)
            n = next
            for scalar in scalars {
                if scalar < n { delta += 1 }
                guard scalar == n else { continue }
                var q = delta
                var k = base
                while true {
                    let t = k <= bias ? tMin : (k >= bias + tMax ? tMax : k - bias)
                    if q < t { break }
                    output.append(digit(t + (q - t) % (base - t)))
                    q = (q - t) / (base - t)
                    k += base
                }
                output.append(digit(q))
                bias = adapt(delta, points: handled + 1, first: handled == basicCount)
                delta = 0
                handled += 1
            }
            delta += 1
            n += 1
        }
        return String(output)
    }

    private static func digit(_ value: Int) -> Character {
        Character(UnicodeScalar(UInt8(value < 26 ? value + 97 : value + 22)))
    }

    private static func adapt(_ delta: Int, points: Int, first: Bool) -> Int {
        var delta = first ? delta / damp : delta / 2
        delta += delta / points
        var k = 0
        while delta > ((base - tMin) * tMax) / 2 {
            delta /= base - tMin
            k += base
        }
        return k + (base - tMin + 1) * delta / (delta + skew)
    }
}
