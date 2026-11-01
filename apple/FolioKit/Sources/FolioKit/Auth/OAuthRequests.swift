import Foundation

/// Builds the requests of the authorization code flow so they can be checked without a server.
public enum OAuthRequests {
    public static func registration(endpoint: URL, redirectURI: String, clientName: String) throws -> URLRequest {
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(
            withJSONObject: [
                "client_name": clientName,
                "redirect_uris": [redirectURI],
                "token_endpoint_auth_method": "none",
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
            ], options: [.sortedKeys]
        )
        return request
    }

    public static func authorizationURL(
        metadata: OAuthMetadata, clientID: String, redirectURI: String, resource: String, challenge: String,
        state: String
    ) -> URL {
        var components = URLComponents(url: metadata.authorizationEndpoint, resolvingAgainstBaseURL: false)!
        components.queryItems = [
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "client_id", value: clientID),
            URLQueryItem(name: "redirect_uri", value: redirectURI),
            URLQueryItem(name: "code_challenge", value: challenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
            URLQueryItem(name: "scope", value: OAuthConfiguration.scopes),
            URLQueryItem(name: "resource", value: resource),
            URLQueryItem(name: "state", value: state),
        ]
        return components.url!
    }

    public static func tokenExchange(
        endpoint: URL, clientID: String, code: String, verifier: String, redirectURI: String, resource: String
    ) -> URLRequest {
        form(
            endpoint,
            [
                ("grant_type", "authorization_code"), ("client_id", clientID), ("code", code),
                ("code_verifier", verifier), ("redirect_uri", redirectURI), ("resource", resource),
            ]
        )
    }

    public static func refresh(endpoint: URL, clientID: String, refreshToken: String) -> URLRequest {
        form(
            endpoint, [("grant_type", "refresh_token"), ("client_id", clientID), ("refresh_token", refreshToken)]
        )
    }

    public static func revocation(endpoint: URL, clientID: String, token: String) -> URLRequest {
        form(endpoint, [("token", token), ("client_id", clientID)])
    }

    private static func form(_ endpoint: URL, _ fields: [(String, String)]) -> URLRequest {
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        let unreserved = CharacterSet(
            charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
        )
        request.httpBody = Data(
            fields.map { name, value in
                "\(name)=\(value.addingPercentEncoding(withAllowedCharacters: unreserved) ?? "")"
            }.joined(separator: "&").utf8
        )
        return request
    }
}
