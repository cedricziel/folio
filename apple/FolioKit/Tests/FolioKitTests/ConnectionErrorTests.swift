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
        .serverCertificateNotYetValid,
    ])
    func certificateProblemsNameTheHost(code: URLError.Code) {
        let mapped = FolioError(transport: error(code))
        #expect(mapped == .untrustedCertificate(host: "folio.example.com"))
        #expect(
            mapped.localizedDescription
                == "folio.example.com doesn't have a certificate this device trusts, so Folio won't connect. "
                + "Check the address, or ask whoever runs the server to fix its certificate.")
    }

    @Test func failedSecureConnectionPointsAtAddressAndPort() {
        let mapped = FolioError(transport: error(.secureConnectionFailed))
        #expect(mapped == .secureConnectionFailed(host: "folio.example.com"))
        #expect(
            mapped.localizedDescription
                == "Couldn't make a secure connection to folio.example.com. Check that the address and port are "
                + "right and the server uses HTTPS.")
    }

    @Test(arguments: [URLError.Code.clientCertificateRejected, .clientCertificateRequired])
    func clientCertificateProblemsBlameThisDevice(code: URLError.Code) {
        let mapped = FolioError(transport: error(code))
        #expect(mapped == .clientCertificate(host: "folio.example.com"))
        #expect(
            mapped.localizedDescription
                == "folio.example.com wants a certificate from this device, and it has none the server accepts. "
                + "Install the certificate your administrator gave you.")
    }

    @Test(arguments: [URLError.Code.internationalRoamingOff, .callIsActive])
    func roamingAndCallsCountAsOffline(code: URLError.Code) {
        #expect(FolioError(transport: error(code)) == .offline)
    }

    @Test(arguments: [URLError.Code.badURL, .unsupportedURL])
    func badAddressesAreInvalidServerURLs(code: URLError.Code) {
        #expect(FolioError(transport: error(code)) == .invalidServerURL)
    }

    @Test(arguments: [URLError.Code.badServerResponse, .cannotParseResponse])
    func garbledAnswersAreUnreadable(code: URLError.Code) {
        #expect(FolioError(transport: error(code)) == .unreadableResponse)
    }

    @Test(arguments: [URLError.Code.httpTooManyRedirects, .redirectToNonExistentLocation])
    func redirectProblemsHaveTheirOwnMessage(code: URLError.Code) {
        let mapped = FolioError(transport: error(code))
        #expect(mapped == .redirectLoop(host: "folio.example.com"))
        #expect(
            mapped.localizedDescription
                == "folio.example.com keeps redirecting somewhere that doesn't work. Check the address.")
    }

    @Test func proxySignInHasItsOwnMessage() {
        let mapped = FolioError(transport: error(.userAuthenticationRequired))
        #expect(mapped == .networkSignInRequired)
        #expect(mapped.localizedDescription == "This network asks you to sign in first. Sign in there, then try again.")
    }

    @Test func nonDefaultPortsAreShown() {
        let url = URL(string: "https://nas.local:8443/api?token=secret")!
        let withPort = URLError(.cannotConnectToHost, userInfo: [NSURLErrorFailingURLErrorKey: url])
        #expect(FolioError(transport: withPort) == .hostUnreachable(host: "nas.local:8443"))
        let standard = URLError(
            .cannotConnectToHost, userInfo: [NSURLErrorFailingURLErrorKey: URL(string: "https://nas.local:443/")!])
        #expect(FolioError(transport: standard) == .hostUnreachable(host: "nas.local"))
        let http = URLError(
            .cannotConnectToHost, userInfo: [NSURLErrorFailingURLErrorKey: URL(string: "http://nas.local:80/")!])
        #expect(FolioError(transport: http) == .hostUnreachable(host: "nas.local"))
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
        #expect(FolioError(transport: error(.dnsLookupFailed)) != .unreachable)
        #expect(FolioError(transport: error(.cannotCreateFile)) == .unreachable)
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
            .untrustedCertificate(host: "h"), .secureConnectionFailed(host: "h"), .clientCertificate(host: "h"),
            .redirectLoop(host: "h"), .networkSignInRequired, .insecureAddress, .unreadableResponse, .signInRequired,
            .server(status: 404, message: nil), .invalidServerURL,
        ]
        for error in permanent { #expect(!error.isRetryable, "\(error)") }
    }

    @Test func connectionFailuresArePartitionedByRetryability() {
        #expect(FolioError.offline.isConnectionFailure)
        #expect(FolioError.insecureAddress.isConnectionFailure)
        #expect(!FolioError.signInRequired.isConnectionFailure)
        #expect(!FolioError.server(status: 503, message: nil).isConnectionFailure)
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

private final class FailingURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var code = URLError.Code.unknown
    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let info = [NSURLErrorFailingURLErrorKey: request.url as Any]
        client?.urlProtocol(self, didFailWithError: URLError(Self.code, userInfo: info))
    }
    override func stopLoading() {}
}
