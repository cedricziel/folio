import AuthenticationServices
import XCTest

final class WebAuthenticatorTests: XCTestCase {
    @MainActor
    func testCallbackCanCompleteOnBackgroundQueue() async throws {
        let expected = URL(string: "com.cedricziel.folio:/oauth/callback?code=test")!
        let actual = try await authenticate(callback: expected)
        XCTAssertEqual(actual, expected)
    }

    @MainActor
    func testCancellationReturnsSignInFailure() async {
        do {
            _ = try await authenticate(error: ASWebAuthenticationSessionError(.canceledLogin))
            XCTFail("Expected cancellation to fail sign-in")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Sign-in was cancelled or denied.")
        }
    }

    @MainActor
    func testUnderlyingErrorIsPreserved() async {
        let expected = NSError(domain: "AuthenticationTest", code: 42)
        do {
            _ = try await authenticate(error: expected)
            XCTFail("Expected authentication error")
        } catch {
            XCTAssertEqual(error as NSError, expected)
        }
    }

    @MainActor
    func testMissingCallbackAndErrorReturnsSignInFailure() async {
        do {
            _ = try await authenticate()
            XCTFail("Expected missing callback to fail sign-in")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Sign-in failed.")
        }
    }

    @MainActor
    private func authenticate(callback: URL? = nil, error: (any Error)? = nil) async throws -> URL {
        let authenticator = SystemWebAuthenticator { _, _, completion in
            BackgroundSession(completion: completion, callback: callback, error: error)
        }
        return try await authenticator.authenticate(
            url: URL(string: "https://example.invalid/authorize")!, callbackScheme: "com.cedricziel.folio"
        )
    }
}

@MainActor
private final class BackgroundSession: WebAuthenticationSession {
    var presentationContextProvider: (any ASWebAuthenticationPresentationContextProviding)?
    private let completion: Completion
    private let callback: URL?
    private let error: (any Error)?

    init(completion: @escaping (URL?, (any Error)?) -> Void, callback: URL?, error: (any Error)?) {
        self.completion = Completion(call: completion)
        self.callback = callback
        self.error = error
    }

    func start() -> Bool {
        let completion = completion
        let callback = callback
        let error = error
        DispatchQueue.global().async {
            completion.call(callback, error)
        }
        return true
    }
}

// Model the framework's non-Sendable callback crossing its XPC queue boundary.
private struct Completion: @unchecked Sendable {
    let call: (URL?, (any Error)?) -> Void
}
