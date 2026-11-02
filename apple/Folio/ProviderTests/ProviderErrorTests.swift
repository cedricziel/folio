import FileProvider
import FolioKit
import XCTest

final class ProviderErrorTests: XCTestCase {
    func testOfflineFailureUsesFileProviderDomain() {
        let error = FolioError.unreachable.providerError as NSError
        XCTAssertEqual(error.domain, NSFileProviderErrorDomain)
        XCTAssertEqual(error.code, NSFileProviderError.serverUnreachable.rawValue)
    }

    func testMissingOriginalReportsNoSuchItem() {
        let error = FolioError.server(status: 404, message: "Missing original").providerError as NSError
        XCTAssertEqual(error.domain, NSFileProviderErrorDomain)
        XCTAssertEqual(error.code, NSFileProviderError.noSuchItem.rawValue)
    }

    func testAuthenticationFailureRequestsSignIn() {
        XCTAssertEqual((FolioError.signInRequired.providerError as? NSFileProviderError)?.code, .notAuthenticated)
    }

    func testPermissionFailureUsesCocoaReadPermissionError() {
        let error = FolioError.server(status: 403, message: nil).providerError as NSError
        XCTAssertEqual(error.domain, NSCocoaErrorDomain)
        XCTAssertEqual(error.code, NSFileReadNoPermissionError)
    }

    func testCancellationUsesCocoaCancellationError() {
        let error = CancellationError().providerError as NSError
        XCTAssertEqual(error.domain, NSCocoaErrorDomain)
        XCTAssertEqual(error.code, NSUserCancelledError)
    }

    func testUnknownErrorPreservesCauseInSupportedDomain() {
        let original = FolioError.unreadableResponse
        let error = original.providerError as NSError
        XCTAssertEqual(error.domain, NSCocoaErrorDomain)
        XCTAssertEqual(error.code, NSXPCConnectionReplyInvalid)
        XCTAssertEqual(error.userInfo[NSUnderlyingErrorKey] as? FolioError, original)
    }
}
