import AuthenticationServices
import FolioKit

/// Runs the authorization page in `ASWebAuthenticationSession`, which shares cookies with the system browser,
/// so a Folio web login carries over.
@MainActor
final class SystemWebAuthenticator: NSObject, WebAuthenticating, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?

    nonisolated func authenticate(url: URL, callbackScheme: String) async throws -> URL {
        try await begin(url: url, callbackScheme: callbackScheme)
    }

    private func begin(url: URL, callbackScheme: String) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(url: url, callback: .customScheme(callbackScheme)) {
                callback, error in
                if let callback {
                    continuation.resume(returning: callback)
                } else if (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin {
                    continuation.resume(throwing: FolioError.signInFailed("Sign-in was cancelled or denied."))
                } else {
                    continuation.resume(throwing: error ?? FolioError.signInFailed("Sign-in failed."))
                }
            }
            session.presentationContextProvider = self
            // The session only lives while something holds it.
            self.session = session
            session.start()
        }
    }

    func presentationAnchor(for _: ASWebAuthenticationSession) -> ASPresentationAnchor {
        #if os(iOS)
            return UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
                ?? ASPresentationAnchor()
        #else
            return NSApplication.shared.keyWindow ?? ASPresentationAnchor()
        #endif
    }
}
