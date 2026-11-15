import Foundation
import Testing

@testable import FolioKit

struct ConnectionErrorTests {
    private func error(_ code: URLError.Code, host: String = "folio.example.com") -> URLError {
        URLError(code, userInfo: [NSURLErrorFailingURLErrorKey: URL(string: "https://\(host)/api?token=secret")!])
    }

    @Test(arguments: [URLError.Code.notConnectedToInternet, .networkConnectionLost, .dataNotAllowed])
    func offlineCodesSayOffline(code: URLError.Code) {
        let mapped = FolioError(transport: error(code))
        #expect(mapped == .offline)
        #expect(mapped.localizedDescription == "You're offline. Try again when you're connected.")
    }

    @Test(arguments: [URLError.Code.cannotFindHost, .dnsLookupFailed, .cannotConnectToHost])
    func unreachableHostNamesTheHost(code: URLError.Code) {
        let mapped = FolioError(transport: error(code))
        #expect(mapped == .hostUnreachable(host: "folio.example.com"))
        #expect(
            mapped.localizedDescription
                == "Couldn't connect to folio.example.com. Check the address, or the server may be down.")
    }

    @Test func timeoutSaysTheServerWasSlow() {
        let mapped = FolioError(transport: error(.timedOut))
        #expect(mapped == .timedOut)
        #expect(mapped.localizedDescription == "The server took too long to answer. Try again.")
    }

    @Test(arguments: [
        URLError.Code.serverCertificateUntrusted, .serverCertificateHasBadDate, .serverCertificateHasUnknownRoot,
        .serverCertificateNotYetValid, .secureConnectionFailed, .clientCertificateRejected,
    ])
    func certificateProblemsNameTheHost(code: URLError.Code) {
        let mapped = FolioError(transport: error(code))
        #expect(mapped == .untrustedCertificate(host: "folio.example.com"))
        #expect(
            mapped.localizedDescription
                == "folio.example.com doesn't have a certificate this device trusts, so Folio won't connect.")
    }

    @Test func blockedHTTPAddressSaysUseHTTPS() {
        let mapped = FolioError(transport: error(.appTransportSecurityRequiresSecureConnection))
        #expect(mapped == .insecureAddress)
        #expect(mapped.localizedDescription == "Folio can't use http:// addresses. Use an https:// address instead.")
    }

    @Test func messagesNeverCarryURLsCodesOrQueries() {
        for code in [URLError.Code.cannotFindHost, .serverCertificateUntrusted, .timedOut, .badURL, .unknown] {
            let text = FolioError(transport: error(code)).localizedDescription
            #expect(!text.contains("secret"))
            #expect(!text.contains("?"))
            #expect(!text.contains("NSURLErrorDomain"))
            #expect(!text.contains("-1"))
        }
    }

    @Test func otherURLErrorsStayGenericallyUnreachable() {
        #expect(FolioError(transport: error(.badServerResponse)) == .unreachable)
        #expect(FolioError(transport: error(.cancelled)) == .unreachable)
    }

    @Test func serverErrorsWithoutMessageAdviseWaiting() {
        #expect(
            FolioError.server(status: 503, message: nil).localizedDescription
                == "The server had a problem. Try again later.")
        #expect(
            FolioError.server(status: 404, message: nil).localizedDescription == "The server answered with status 404.")
    }

    @Test func signInRequiredSaysSignInAgain() {
        #expect(FolioError.signInRequired.localizedDescription == "Your session has expired. Sign in again.")
    }

    @Test func causesThatCanFixThemselvesAreRetryable() {
        let retryable: [FolioError] = [
            .unreachable, .offline, .timedOut, .hostUnreachable(host: "h"),
            .server(status: 503, message: nil), .server(status: 408, message: nil), .server(status: 429, message: nil),
        ]
        for error in retryable { #expect(error.isRetryable, "\(error)") }
        let permanent: [FolioError] = [
            .untrustedCertificate(host: "h"), .insecureAddress, .signInRequired,
            .server(status: 404, message: nil), .invalidServerURL,
        ]
        for error in permanent { #expect(!error.isRetryable, "\(error)") }
    }

    @Test func liveServiceMapsTransportFailures() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [FailingURLProtocol.self]
        FailingURLProtocol.code = .notConnectedToInternet
        let service = LiveFolioService(
            baseURL: URL(string: "https://folio.example.com")!, session: URLSession(configuration: config))
        await #expect(throws: FolioError.offline) { _ = try await service.correspondents() }
    }
}

final class FailingURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var code = URLError.Code.unknown
    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        client?.urlProtocol(self, didFailWithError: URLError(Self.code, userInfo: [NSURLErrorFailingURLErrorKey: request.url as Any]))
    }
    override func stopLoading() {}
}
