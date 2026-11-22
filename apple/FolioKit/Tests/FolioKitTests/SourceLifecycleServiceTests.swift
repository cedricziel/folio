import Foundation
import Testing

@testable import FolioKit

@Suite(.serialized) struct SourceLifecycleServiceTests {
    @Test func deletedListKeepsVaultAndLifecycleWithoutCredentialsOrDiagnostics() async throws {
        let vault = UUID(), source = UUID()
        SourceLifecycleURLProtocol.handler = { request in
            #expect(request.url?.query == "deleted=true")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            return (200, Self.json(vault: vault, source: source, deleted: true))
        }
        let rows = try await service().sourceActivity(vaultID: vault, deleted: true)
        #expect(rows.first?.createdBy == "fixture-creator")
        #expect(rows.first?.updatedBy == "fixture-editor")
        #expect(rows.first?.deletedAt != nil)
        #expect(rows.first?.supportsLifecycle == true)
        #expect(!String(describing: rows).contains("private"))
    }

    @Test func enablingFetchesCurrentSettingsAndPreservesMaskedSecret() async throws {
        let vault = UUID(), source = UUID()
        SourceLifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/api/sources/\(source.uuidString.lowercased())")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            if request.httpMethod == "PUT" {
                let body = request.httpBody.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
                #expect(body?["enabled"] as? Bool == false)
                #expect(body?["name"] as? String == "Mailbox")
                #expect((body?["config"] as? [String: Any])?["password"] as? String == "********")
                #expect(body?.count == 3)
                return (200, Self.json(vault: vault, source: source, enabled: false))
            }
            return (200, Self.json(vault: vault, source: source))
        }
        let updated = try await service().setSourceEnabled(vaultID: vault, sourceID: source, enabled: false)
        #expect(!updated.enabled)
        #expect(updated.health == .off)
    }

    @Test func foreignVaultSettingsCannotBeWritten() async {
        SourceLifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            return (200, Self.json(vault: UUID(), source: UUID()))
        }
        await #expect(throws: FolioError.unreadableResponse) {
            try await service().setSourceEnabled(vaultID: UUID(), sourceID: UUID(), enabled: false)
        }
    }

    @Test func configuredSourceCannotBeWritten() async {
        let vault = UUID(), source = UUID()
        SourceLifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            return (200, Self.json(vault: vault, source: source).replacingOccurrences(of: "managed", with: "config"))
        }
        await #expect(throws: FolioError.unreadableResponse) {
            try await service().setSourceEnabled(vaultID: vault, sourceID: source, enabled: false)
        }
    }

    @Test func trashUsesScopedDeleteAndRestoreUsesScopedPost() async throws {
        let vault = UUID(), source = UUID()
        SourceLifecycleURLProtocol.handler = { request in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            #expect(request.httpMethod == "DELETE")
            #expect(request.url?.path == "/api/sources/\(source.uuidString.lowercased())")
            return (204, "")
        }
        try await service().trashSource(vaultID: vault, sourceID: source)
        SourceLifecycleURLProtocol.handler = { request in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vault.uuidString.lowercased())
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/api/sources/\(source.uuidString.lowercased())/restore")
            return (200, Self.json(vault: vault, source: source))
        }
        let restored = try await service().restoreSource(vaultID: vault, sourceID: source)
        #expect(restored.id == source)
        #expect(restored.deletedAt == nil)
    }

    private func service() -> LiveFolioService {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SourceLifecycleURLProtocol.self]
        return LiveFolioService(
            baseURL: URL(string: "https://source-lifecycle.test")!, session: URLSession(configuration: configuration))
    }

    private static func json(vault: UUID, source: UUID, enabled: Bool = true, deleted: Bool = false) -> String {
        let json = """
            {"id":"\(source)","vault_id":"\(vault)","name":"Mailbox","enabled":\(enabled),"origin":"managed",
            "created_at":"2026-11-01T10:00:00Z","updated_at":"2026-11-02T10:00:00Z",
            "deleted_at":\(deleted ? "\"2026-11-03T10:00:00Z\"" : "null"),"created_by":"fixture-creator","updated_by":"fixture-editor",
            "config":{"type":"email","password":"********","host":"private.test"},"status":{"last_error":"private"}}
            """
        return deleted ? "[\(json)]" : json
    }
}

private final class SourceLifecycleURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: ((URLRequest) -> (Int, String))?
    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let (status, body) = Self.handler?(request) ?? (500, "")
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
