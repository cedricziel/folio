import Foundation

/// The server's web pages for handed-over targets, which are also the universal links a self-built app
/// accepts: `{server}/vaults/{vault}/documents/{id}`, `{server}/vaults/{vault}/` (Inbox, or All documents
/// with `?view=all`) and `{server}/vaults/{vault}/saved-views/{id}`. `{server}` keeps its path prefix.
public enum HandoffLink {
    public static func webURL(for payload: HandoffPayload) -> URL? {
        guard var components = URLComponents(url: payload.server, resolvingAgainstBaseURL: false) else {
            return nil
        }
        var prefix = components.percentEncodedPath
        while prefix.hasSuffix("/") { prefix.removeLast() }
        let vault = "\(prefix)/vaults/\(payload.vaultID.uuidString.lowercased())"
        components.user = nil
        components.password = nil
        components.query = nil
        components.fragment = nil
        switch payload.target {
        case let .document(id, _):
            components.percentEncodedPath = "\(vault)/documents/\(id.uuidString.lowercased())"
        case let .library(list):
            components.percentEncodedPath = "\(vault)/"
            if list == .all { components.query = "view=all" }
        case let .savedView(id):
            components.percentEncodedPath = "\(vault)/saved-views/\(id.uuidString.lowercased())"
        }
        return components.url
    }

    /// Reads a web page link back into a payload. The last `/vaults/{uuid}` pair splits the server's
    /// path prefix from the target; any other page shape is not a handed-over target.
    public static func parse(_ url: URL) -> HandoffPayload? {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
            let scheme = components.scheme?.lowercased(), scheme == "http" || scheme == "https",
            let host = components.percentEncodedHost, !host.isEmpty, components.percentEncodedPath.hasPrefix("/")
        else { return nil }
        let segments = components.percentEncodedPath.dropFirst().split(separator: "/", omittingEmptySubsequences: false)
        guard
            let index = segments.indices.dropLast().last(where: {
                segments[$0] == "vaults" && UUID(uuidString: String(segments[$0 + 1])) != nil
            }),
            let vault = UUID(uuidString: String(segments[index + 1]))
        else { return nil }
        let prefix = segments[..<index]
        guard !prefix.contains(where: \.isEmpty) else { return nil }
        let tail = Array(segments[(index + 2)...])

        let target: HandoffTarget
        switch tail {
        case [], [""]:
            let all = components.queryItems?.contains { $0.name == "view" && $0.value == "all" } ?? false
            target = .library(all ? .all : .inbox)
        case let pair where pair.count == 2 && pair[0] == "documents":
            guard let id = UUID(uuidString: String(pair[1])) else { return nil }
            target = .document(id)
        case let pair where pair.count == 2 && pair[0] == "saved-views":
            guard let id = UUID(uuidString: String(pair[1])) else { return nil }
            target = .savedView(id)
        default:
            return nil
        }

        var server = URLComponents()
        server.scheme = scheme
        server.percentEncodedHost = host
        server.port = components.port
        server.percentEncodedPath = prefix.isEmpty ? "" : "/" + prefix.joined(separator: "/")
        guard let serverURL = server.url else { return nil }
        return HandoffPayload(server: serverURL, vaultID: vault, target: target)
    }
}
