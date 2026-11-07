import Foundation
import Security

/// What the app and every extension agree on: where the server is and how to authenticate.
public enum SharedConfiguration {
    public static let appGroup = UserDefaultsServerConfigStore.appGroup

    /// Info.plist key holding `$(AppIdentifierPrefix)com.cedricziel.folio`, which is only known at build time.
    public static var keychainAccessGroup: String? {
        Bundle.main.object(forInfoDictionaryKey: "FolioKeychainGroup") as? String
    }

    public static func serverURL() -> URL? {
        UserDefaultsServerConfigStore.shared().load()
    }

    public static func makeService(session: URLSession? = nil, downloadDirectory: URL? = nil) -> LiveFolioService? {
        serverURL().map { makeService(for: $0, session: session, downloadDirectory: downloadDirectory) }
    }

    public static func makeService(
        for url: URL, session: URLSession? = nil, downloadDirectory: URL? = nil
    ) -> LiveFolioService {
        let provider = tokenProvider(for: url)
        FolioTelemetry.shared.start(baseURL: url, tokenProvider: provider)
        return LiveFolioService(
            baseURL: url, tokenProvider: provider, session: session, telemetry: .shared,
            downloadDirectory: downloadDirectory)

    }

    /// Coordinates credential writes and restoration across the app and its extensions.
    public static let oauthRefreshLock = OAuthRefreshLock(
        url: container?.appending(path: "folio-oauth-refresh.lock"))

    private static let sharedOAuth = OAuthSession(
        store: KeychainCredentialStore(),
        refreshLock: oauthRefreshLock
    )

    /// One session per process, so concurrent requests share a single refresh.
    public static func tokenProvider() -> OAuthSession { sharedOAuth }

    /// The shared session's tokens, withheld from any server other than the one that issued them.
    public static func tokenProvider(for server: URL) -> ServerBoundTokenProvider {
        ServerBoundTokenProvider(session: sharedOAuth, server: server)
    }

    public static var container: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
    }
}
