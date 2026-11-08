import Foundation
import Testing

@testable import FolioKit

/// Answers device requests; separate from `StubURLProtocol` so suites can run in parallel.
final class DeviceStubProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var requests: [(URLRequest, Data?)] = []
    nonisolated(unsafe) static var reply: (status: Int, body: String) = (200, "")

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        var body = request.httpBody
        if body == nil, let stream = request.httpBodyStream {
            stream.open()
            var data = Data()
            var buffer = [UInt8](repeating: 0, count: 4096)
            while true {
                let count = stream.read(&buffer, maxLength: buffer.count)
                if count <= 0 { break }
                data.append(buffer, count: count)
            }
            stream.close()
            body = data
        }
        Self.requests.append((request, body))
        let response = HTTPURLResponse(
            url: request.url!, statusCode: Self.reply.status, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(Self.reply.body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

private struct StaticToken: BearerTokenProvider {
    func token() async -> String? { "app-token" }
}

private func liveService() -> LiveFolioService {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [DeviceStubProtocol.self]
    return LiveFolioService(
        baseURL: URL(string: "https://folio.test")!, tokenProvider: StaticToken(),
        session: URLSession(configuration: configuration)
    )
}

private let registeredJSON = """
    {"device": {"installation_id": "install-1", "platform": "ios", "environment": "production",
      "locale": "de-DE", "has_alert_token": true, "has_widget_token": false,
      "created_at": "2026-11-08T10:00:00Z", "updated_at": "2026-11-08T10:00:00Z", "last_delivered_at": null,
      "preferences": {"processed": false, "needs_review": true, "failed": true, "scope": "mine",
        "vaults": ["aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"], "show_titles": true}},
     "push_available": true}
    """

@Suite(.serialized) struct DeviceRegistrationServiceTests {
    @Test func registrationPutsTheTokenAndPreferences() async throws {
        DeviceStubProtocol.requests = []
        DeviceStubProtocol.reply = (201, registeredJSON)
        let registration = DeviceRegistration(
            platform: .ios, environment: .production, locale: "de-DE", alertToken: "abcd",
            preferences: NotificationPreferences(scope: .editable)
        )

        let result = try await liveService().registerDevice(installationID: "install-1", registration)

        #expect(result.pushAvailable)
        #expect(result.preferences.vaults == .only([UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!]))
        #expect(result.preferences.showTitles)
        let (request, body) = try #require(DeviceStubProtocol.requests.first)
        #expect(request.httpMethod == "PUT")
        #expect(request.url?.path == "/api/devices/install-1")
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer app-token")
        let json = try #require(body.flatMap { try JSONSerialization.jsonObject(with: $0) as? [String: Any] })
        #expect(json["platform"] as? String == "ios")
        #expect(json["environment"] as? String == "production")
        #expect(json["alert_token"] as? String == "abcd")
        #expect(json["widget_token"] == nil)
        let preferences = try #require(json["preferences"] as? [String: Any])
        #expect(preferences["scope"] as? String == "editable")
        #expect(preferences["vaults"] as? String == "all")
        #expect(preferences["needs_review"] as? Bool == true)
    }

    @Test func deletionAndTestUseTheInstallationPath() async throws {
        DeviceStubProtocol.requests = []
        DeviceStubProtocol.reply = (204, "")
        try await liveService().deleteDevice(installationID: "install-1")
        DeviceStubProtocol.reply = (202, "")
        try await liveService().sendTestNotification(installationID: "install-1")

        #expect(DeviceStubProtocol.requests.map { $0.0.httpMethod } == ["DELETE", "POST"])
        #expect(DeviceStubProtocol.requests.map { $0.0.url?.path } == [
            "/api/devices/install-1", "/api/devices/install-1/test",
        ])
    }

    @Test func pushNotConfiguredSurfacesTheServerMessage() async throws {
        DeviceStubProtocol.reply = (409, #"{"error": "Push notifications are not configured on this server"}"#)
        await #expect(throws: FolioError.server(
            status: 409, message: "Push notifications are not configured on this server"
        )) {
            try await liveService().sendTestNotification(installationID: "install-1")
        }
    }
}

struct NotificationPayloadTests {
    @Test func oneDocumentOpensIt() throws {
        let document = UUID()
        let vault = UUID()
        let note = try #require(FolioNotification(userInfo: [
            "aps": ["alert": ["title": "Folio", "body": "1 document needs review"]],
            "folio": [
                "v": 1, "document": document.uuidString.lowercased(), "vault": vault.uuidString.lowercased(),
                "counts": ["needs_review": 1, "failed": 0, "processed": 0],
            ],
        ]))
        #expect(note.route == .document(document, vaultID: vault))
        #expect(note.canMarkReviewed)
    }

    @Test func severalDocumentsOpenTheInbox() throws {
        let note = try #require(FolioNotification(userInfo: [
            "folio": ["v": 1, "document": NSNull(), "vault": NSNull(), "counts": ["needs_review": 3, "failed": 1]]
        ]))
        #expect(note.route == .inbox)
        #expect(!note.canMarkReviewed)
        #expect(note.needsReview == 3)
        #expect(note.failed == 1)
    }

    @Test func aFailedDocumentCannotBeMarkedReviewed() throws {
        let note = try #require(FolioNotification(userInfo: [
            "folio": ["v": 1, "document": UUID().uuidString, "counts": ["failed": 1]]
        ]))
        #expect(!note.canMarkReviewed)
    }

    @Test func foreignAndNewerPayloadsAreIgnored() {
        #expect(FolioNotification(userInfo: ["aps": ["alert": "hi"]]) == nil)
        #expect(FolioNotification(userInfo: ["folio": ["v": 2]]) == nil)
    }

    @Test func tokensAreLowercaseHex() {
        #expect(PushToken.hex(Data([0x0A, 0xBC, 0xFF])) == "0abcff")
    }

    @Test func preferencesDefaultToTheUploadersReviewAndFailures() throws {
        let decoded = try JSONDecoder().decode(NotificationPreferences.self, from: Data("{}".utf8))
        #expect(decoded == NotificationPreferences())
        #expect(decoded.needsReview && decoded.failed && !decoded.processed && !decoded.showTitles)
        #expect(decoded.scope == .mine && decoded.vaults == .all)
    }
}

/// Records calls; can hold registration until released, to test races.
private actor FakeDeviceService: DeviceRegistrationService {
    private(set) var calls: [String] = []
    private(set) var registrations: [DeviceRegistration] = []
    var available = true
    private var gate: CheckedContinuation<Void, Never>?
    private var holding = false

    func hold() { holding = true }

    func release() {
        holding = false
        gate?.resume()
        gate = nil
    }

    func registerDevice(installationID: String, _ registration: DeviceRegistration) async throws
        -> DeviceRegistrationResult
    {
        calls.append("register \(installationID)")
        registrations.append(registration)
        if holding {
            await withCheckedContinuation { gate = $0 }
        }
        return DeviceRegistrationResult(preferences: registration.preferences, pushAvailable: available)
    }

    func deleteDevice(installationID: String) async throws {
        calls.append("delete \(installationID)")
    }

    func sendTestNotification(installationID: String) async throws {
        calls.append("test \(installationID)")
    }
}

@MainActor struct PushRegistrationTests {
    private let server = URL(string: "https://folio.test")!

    private func make(_ service: FakeDeviceService) -> (PushRegistration, UserDefaults) {
        let defaults = UserDefaults(suiteName: "push-tests-\(UUID().uuidString)")!
        let registration = PushRegistration(
            defaults: defaults, installationID: { "install-1" }, service: { _ in service }
        )
        return (registration, defaults)
    }

    @Test func registersOnceTheTokenAndServerAreKnown() async {
        let service = FakeDeviceService()
        let (push, _) = make(service)

        await push.received(token: Data([0xAB, 0xCD]))
        #expect(await service.calls.isEmpty, "not signed in yet")
        await push.signedIn(to: server)

        #expect(await service.calls == ["register install-1"])
        #expect(await service.registrations.first?.alertToken == "abcd")
        #expect(push.pushAvailable == true)

        await push.received(token: Data([0xAB, 0xCD]))
        #expect(await service.calls.count == 1, "the same token is not sent again")
    }

    @Test func preferencesPersistAndReRegister() async {
        let service = FakeDeviceService()
        let (push, defaults) = make(service)
        await push.signedIn(to: server)
        await push.received(token: Data([0x01]))

        await push.update(NotificationPreferences(processed: true, showTitles: true))

        #expect(await service.registrations.last?.preferences.showTitles == true)
        let reloaded = PushRegistration(defaults: defaults, installationID: { "install-1" }, service: { _ in service })
        #expect(reloaded.preferences.processed)
    }

    @Test func signingOutDeletesTheDevice() async {
        let service = FakeDeviceService()
        let (push, _) = make(service)
        await push.signedIn(to: server)
        await push.received(token: Data([0x01]))

        await push.signingOut()

        #expect(await service.calls.last == "delete install-1")
        #expect(push.pushAvailable == nil)
    }

    @Test func aSignOutDuringRegistrationWins() async {
        let service = FakeDeviceService()
        let (push, _) = make(service)
        await push.signedIn(to: server)
        await service.hold()

        let registering = Task { await push.received(token: Data([0x01])) }
        while await service.calls.isEmpty { await Task.yield() }
        await push.signingOut()
        await service.release()
        await registering.value

        #expect(push.pushAvailable == nil)
    }

    @Test func unavailablePushIsReported() async {
        let service = FakeDeviceService()
        await service.setUnavailable()
        let (push, _) = make(service)
        await push.received(token: Data([0x01]))
        await push.signedIn(to: server)

        #expect(push.pushAvailable == false)
    }
}

private extension FakeDeviceService {
    func setUnavailable() { available = false }
}
