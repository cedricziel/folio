import CryptoKit
import Foundation

public enum OAuthConfiguration {
    public static let clientName = "Folio"
    public static var callbackScheme: String { AppConfiguration.callbackScheme }
    public static var redirectURI: String { AppConfiguration.redirectURI(scheme: callbackScheme) }
    public static let scopes = "documents:read documents:write"
}

/// RFC 7636 proof key for code exchange, S256 only.
public struct PKCE: Equatable, Sendable {
    public let verifier: String
    public let challenge: String

    public static func generate() -> PKCE {
        let verifier = Data(SymmetricKey(size: .bits256).withUnsafeBytes { Array($0) }).base64URLEncoded
        return PKCE(verifier: verifier, challenge: challenge(for: verifier))
    }

    public static func challenge(for verifier: String) -> String {
        Data(SHA256.hash(data: Data(verifier.utf8))).base64URLEncoded
    }
}

extension Data {
    var base64URLEncoded: String {
        base64EncodedString().replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
}

public struct OAuthTokens: Codable, Equatable, Sendable {
    public var accessToken: String
    public var refreshToken: String?
    public var expiresAt: Date

    public init(accessToken: String, refreshToken: String?, expiresAt: Date) {
        self.accessToken = accessToken
        self.refreshToken = refreshToken
        self.expiresAt = expiresAt
    }

    /// What is left once the server has refused the refresh token.
    public static let signedOut = OAuthTokens(accessToken: "", refreshToken: nil, expiresAt: .distantPast)
}

/// Everything needed to use and refresh a sign-in, kept together so one atomic write rotates it.
public struct OAuthCredentials: Codable, Equatable, Sendable {
    public var serverURL: URL
    public var clientID: String
    public var tokenEndpoint: URL
    public var revocationEndpoint: URL?
    public var tokens: OAuthTokens
    public var refreshPending: Bool?

    public init(
        serverURL: URL, clientID: String, tokenEndpoint: URL, revocationEndpoint: URL?, tokens: OAuthTokens
    ) {
        self.serverURL = serverURL
        self.clientID = clientID
        self.tokenEndpoint = tokenEndpoint
        self.revocationEndpoint = revocationEndpoint
        self.tokens = tokens
    }

    public var isSignedIn: Bool {
        !tokens.accessToken.isEmpty
    }
}

/// RFC 8414 authorization server metadata, the parts Folio uses.
public struct OAuthMetadata: Decodable, Equatable, Sendable {
    public let issuer: String
    public let authorizationEndpoint: URL
    public let tokenEndpoint: URL
    public let registrationEndpoint: URL
    public let revocationEndpoint: URL?

    public init(
        issuer: String, authorizationEndpoint: URL, tokenEndpoint: URL, registrationEndpoint: URL,
        revocationEndpoint: URL?
    ) {
        self.issuer = issuer
        self.authorizationEndpoint = authorizationEndpoint
        self.tokenEndpoint = tokenEndpoint
        self.registrationEndpoint = registrationEndpoint
        self.revocationEndpoint = revocationEndpoint
    }

    private enum CodingKeys: String, CodingKey {
        case issuer
        case authorizationEndpoint = "authorization_endpoint"
        case tokenEndpoint = "token_endpoint"
        case registrationEndpoint = "registration_endpoint"
        case revocationEndpoint = "revocation_endpoint"
    }
}
