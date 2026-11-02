import Foundation

public enum SessionRestoration: Equatable, Sendable {
    case restoring
    case signedOut
    case signedIn(URL)
    case expired(URL)
    case interrupted(URL)
    case unavailable(server: URL?, message: String)

    /// Reads session state after the caller has acquired the shared refresh lock.
    public static func restore(from store: any CredentialStore, server: URL?) -> Self {
        do {
            guard let credentials = try store.load() else { return .signedOut }
            if credentials.refreshPending == true { return .interrupted(credentials.serverURL) }
            return credentials.isSignedIn ? .signedIn(credentials.serverURL) : .expired(credentials.serverURL)
        } catch {
            return .unavailable(server: server, message: error.localizedDescription)
        }
    }
}
