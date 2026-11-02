import AuthenticationServices
import FolioKit

@MainActor
protocol WebAuthenticationSession: AnyObject {
    var presentationContextProvider: (any ASWebAuthenticationPresentationContextProviding)? { get set }
    func start() -> Bool
}

extension ASWebAuthenticationSession: WebAuthenticationSession {}

/// Runs the authorization page in `ASWebAuthenticationSession`, which shares cookies with the system browser,
/// so a Folio web login carries over.
@MainActor
final class SystemWebAuthenticator: NSObject, WebAuthenticating, ASWebAuthenticationPresentationContextProviding {
    typealias SessionFactory =
        @MainActor (
            URL, String, @escaping (URL?, (any Error)?) -> Void
        ) -> any WebAuthenticationSession

    private let makeSession: SessionFactory
    private var session: (any WebAuthenticationSession)?

    init(
        makeSession: @escaping SessionFactory = { url, callbackScheme, completion in
            ASWebAuthenticationSession(
                url: url, callback: .customScheme(callbackScheme), completionHandler: completion
            )
        }
    ) {
        self.makeSession = makeSession
        super.init()
    }

    nonisolated func authenticate(url: URL, callbackScheme: String) async throws -> URL {
        try await begin(url: url, callbackScheme: callbackScheme)
    }

    private func begin(url: URL, callbackScheme: String) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            let session = makeSession(url, callbackScheme, Self.completionHandler(for: continuation))
            session.presentationContextProvider = self
            // The session only lives while something holds it.
            self.session = session
            _ = session.start()
        }
    }

    // macOS can deliver completion on Safari's XPC queue rather than the main actor.
    nonisolated private static func completionHandler(
        for continuation: CheckedContinuation<URL, any Error>
    ) -> @Sendable (URL?, (any Error)?) -> Void {
        { callback, error in
            if let callback {
                continuation.resume(returning: callback)
            } else if (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin {
                continuation.resume(throwing: FolioError.signInFailed("Sign-in was cancelled or denied."))
            } else {
                continuation.resume(throwing: error ?? FolioError.signInFailed("Sign-in failed."))
            }
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
