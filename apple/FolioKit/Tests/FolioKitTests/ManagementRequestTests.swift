import Foundation
import Testing

@testable import FolioKit

@Suite(.serialized) struct ManagementRequestTests {
    @Test func explicitScopeKeepsQueryAndServerBasePath() async throws {
        let vault = UUID()
        ManagementURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/sources")
            #expect(request.url?.query == "deleted=true")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-only")
            return (200, [:], Data("[]".utf8))
        }
        let data = try await service().managementRequest(path: "api/sources?deleted=true", vaultID: vault)
        #expect(data == Data("[]".utf8))
    }

    @Test func accountManagementDoesNotInheritSelectedVault() async throws {
        ManagementURLProtocol.handler = { request in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == nil)
            return (204, [:], Data())
        }
        let selected = service().scoped(to: UUID()) as! LiveFolioService
        _ = try await selected.managementRequest(path: "api/vaults", method: "POST", body: Data("{}".utf8))
    }

    @Test func olderGrantGetsExplicitSignInGuidance() async {
        ManagementURLProtocol.handler = { _ in
            (403, ["WWW-Authenticate": "Bearer error=\"insufficient_scope\", scope=\"vaults:manage\""], Data())
        }
        await #expect(
            throws: FolioError.server(
                status: 403,
                message:
                    "Sign out and sign in again to allow this app to manage vaults. If the server does not offer this permission, update it first."
            )
        ) {
            _ = try await service().managementRequest(path: "api/vaults", method: "POST")
        }
    }

    @Test func ownerDenialKeepsServerReason() async {
        ManagementURLProtocol.handler = { _ in
            (403, [:], Data("{\"error\":\"Only a vault owner can manage this vault\"}".utf8))
        }
        await #expect(throws: FolioError.server(status: 403, message: "Only a vault owner can manage this vault")) {
            _ = try await service().managementRequest(path: "api/vaults", method: "POST")
        }
    }

    private func service() -> LiveFolioService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ManagementURLProtocol.self]
        return LiveFolioService(
            baseURL: URL(string: "https://folio.invalid/folio")!, tokenProvider: ManagementTestToken(),
            session: URLSession(configuration: configuration)
        )
    }
}

private struct ManagementTestToken: BearerTokenProvider {
    func token() async throws -> String? { "test-only" }
}

private final class ManagementURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: ((URLRequest) -> (Int, [String: String], Data))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let (status, headers, data) = Self.handler!(request)
        client?.urlProtocol(
            self,
            didReceive: HTTPURLResponse(
                url: request.url!, statusCode: status, httpVersion: nil,
                headerFields: headers.merging(["Content-Type": "application/json"]) { $1 })!,
            cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
