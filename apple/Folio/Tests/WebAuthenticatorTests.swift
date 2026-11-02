import AuthenticationServices
import XCTest

final class WebAuthenticatorTests: XCTestCase {
    @MainActor
    func testCallbackCanCompleteOnBackgroundQueue() async throws {
        let expected = URL(string: "com.cedricziel.folio:/oauth/callback?code=test")!
        let actual = try await withCheckedThrowingContinuation { continuation in
            let handler = SystemWebAuthenticator.completionHandler(for: continuation)
            DispatchQueue.global().async {
                handler(expected, nil)
            }
        }
        XCTAssertEqual(actual, expected)
    }

    @MainActor
    func testCancellationReturnsSignInFailure() async {
        do {
            let _: URL = try await withCheckedThrowingContinuation { continuation in
                let handler = SystemWebAuthenticator.completionHandler(for: continuation)
                DispatchQueue.global().async {
                    handler(nil, ASWebAuthenticationSessionError(.canceledLogin))
                }
            }
            XCTFail("Expected cancellation to fail sign-in")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Sign-in was cancelled or denied.")
        }
    }

    @MainActor
    func testUnderlyingErrorIsPreserved() async {
        let expected = NSError(domain: "AuthenticationTest", code: 42)
        do {
            let _: URL = try await withCheckedThrowingContinuation { continuation in
                let handler = SystemWebAuthenticator.completionHandler(for: continuation)
                DispatchQueue.global().async {
                    handler(nil, expected)
                }
            }
            XCTFail("Expected authentication error")
        } catch {
            XCTAssertEqual(error as NSError, expected)
        }
    }

    @MainActor
    func testMissingCallbackAndErrorReturnsSignInFailure() async {
        do {
            let _: URL = try await withCheckedThrowingContinuation { continuation in
                let handler = SystemWebAuthenticator.completionHandler(for: continuation)
                DispatchQueue.global().async {
                    handler(nil, nil)
                }
            }
            XCTFail("Expected missing callback to fail sign-in")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Sign-in failed.")
        }
    }
}
