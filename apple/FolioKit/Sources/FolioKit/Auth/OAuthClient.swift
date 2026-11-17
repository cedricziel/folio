import Foundation

enum OAuthFailure: Error, Equatable {
    /// The server refused the refresh token or code for good (revoked, rotated away, expired).
    case invalidGrant
    /// The server refused the request itself (unknown client, unsupported grant). Retrying cannot succeed.
    case refused(code: String, description: String?)
    /// Says nothing about the token: the server or network failed, or something other than the server answered.
    case transient(FolioError)
}

/// Talks to the authorization server. Discovery follows RFC 8414 and RFC 9728, with no hardcoded paths
/// beyond the well-known locations those RFCs define.
struct OAuthClient: Sendable {
    let session: URLSession
    let now: @Sendable () -> Date

    func discover(_ server: URL) async throws -> OAuthMetadata {
        let (data, response) = try await fetch(
            URLRequest(url: server.appending(path: ".well-known/oauth-authorization-server")))
        guard (response as? HTTPURLResponse)?.statusCode == 200,
            let metadata = try? JSONDecoder().decode(OAuthMetadata.self, from: data)
        else { throw FolioError.signInFailed("This server doesn't offer sign-in.") }
        return metadata
    }

    /// The resource indicator for the API: the protected resource document names it, `<server>/api` is the fallback.
    func resource(for server: URL) async -> String {
        let request = URLRequest(url: server.appending(path: ".well-known/oauth-protected-resource/api"))
        if let (data, response) = try? await fetch(request), (response as? HTTPURLResponse)?.statusCode == 200,
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let resource = object["resource"] as? String, !resource.isEmpty
        {
            return resource
        }
        return server.appending(path: "api").absoluteString
    }

    func register(_ metadata: OAuthMetadata) async throws -> String {
        let request = try OAuthRequests.registration(
            endpoint: metadata.registrationEndpoint, redirectURI: OAuthConfiguration.redirectURI,
            clientName: OAuthConfiguration.clientName
        )
        let (data, response) = try await fetch(request)
        guard (200..<300).contains((response as? HTTPURLResponse)?.statusCode ?? 0),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let id = object["client_id"] as? String
        else { throw FolioError.signInFailed("The server didn't accept this app.") }
        return id
    }

    /// RFC 6749 errors where sending the same request again cannot succeed.
    private static let definitiveErrors: Set<String> = [
        "invalid_client", "unauthorized_client", "invalid_request", "unsupported_grant_type", "invalid_scope",
    ]

    func tokens(for request: URLRequest, keeping previousRefresh: String? = nil) async throws -> OAuthTokens {
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch is URLError {
            throw OAuthFailure.transient(.unreachable)
        }
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        guard status == 200, let access = object?["access_token"] as? String else {
            let code = object?["error"] as? String
            if code == "invalid_grant" { throw OAuthFailure.invalidGrant }
            if let code, Self.definitiveErrors.contains(code) {
                throw OAuthFailure.refused(code: code, description: object?["error_description"] as? String)
            }
            // A 4xx or an odd 200 may come from a proxy, so it must not read as an answer from the API.
            let transient = status >= 500 || status == 408 || status == 429
            throw OAuthFailure.transient(transient ? .server(status: status, message: nil) : .unreadableResponse)
        }
        let expiresIn = (object?["expires_in"] as? Double) ?? 3600
        return OAuthTokens(
            accessToken: access, refreshToken: (object?["refresh_token"] as? String) ?? previousRefresh,
            expiresAt: now().addingTimeInterval(expiresIn)
        )
    }

    func revoke(_ request: URLRequest) async {
        _ = try? await fetch(request)
    }

    private func fetch(_ request: URLRequest) async throws -> (Data, URLResponse) {
        do {
            return try await session.data(for: request)
        } catch is URLError {
            throw FolioError.unreachable
        }
    }
}
