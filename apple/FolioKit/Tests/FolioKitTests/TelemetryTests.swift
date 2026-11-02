import Foundation
import OpenTelemetryApi
import OpenTelemetryProtocolExporterCommon
import SwiftProtobuf
import Testing

@testable import FolioKit

private struct SmokeToken: BearerTokenProvider {
    let value: String
    func token() async -> String? {
        value
    }
}

private actor TelemetryToken: BearerTokenProvider {
    var value = "first-token"
    func token() -> String? {
        value
    }

    func refresh(rejecting _: String?) -> String? {
        value = "refreshed-token"; return value
    }
}

private final class TelemetryCapture: @unchecked Sendable {
    let lock = NSLock()
    private var records: [(URLRequest, Data)] = []
    func append(_ request: URLRequest, _ data: Data) {
        lock.withLock { records.append((request, data)) }
    }

    var requests: [(URLRequest, Data)] {
        lock.withLock { records }
    }
}

@Suite(.serialized) struct TelemetryTests {
    private let baseURL = URL(string: "https://folio.example")!

    @Test(.enabled(if: ProcessInfo.processInfo.environment["FOLIO_TELEMETRY_SMOKE_URL"] != nil))
    func realRelaySmoke() async throws {
        let environment = ProcessInfo.processInfo.environment
        let url = try #require(environment["FOLIO_TELEMETRY_SMOKE_URL"].flatMap(URL.init(string:)))
        let suite = "folio-telemetry-smoke-\(UUID())"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let provider = SmokeToken(value: environment["FOLIO_TELEMETRY_SMOKE_TOKEN"] ?? "folio-telemetry-smoke")
        let telemetry = FolioTelemetry(defaults: defaults, installationStore: TestInstallationIDStore())
        defer { telemetry.stop() }
        await telemetry.configure(baseURL: url, tokenProvider: provider)
        let runtime = try #require(telemetry.current)
        let traceID = try await telemetry.operation("folio.swift.smoke") {
            let traceID = try #require(
                OpenTelemetryApi.OpenTelemetry.instance.contextProvider.activeSpan?.context.traceId.hexString
            )
            var request = URLRequest(url: url.appending(path: "api/tags"))
            request.setValue("Bearer \(provider.value)", forHTTPHeaderField: "Authorization")
            let session = URLSession(configuration: .ephemeral, delegate: TelemetryRedirectGuard(), delegateQueue: nil)
            defer { session.invalidateAndCancel() }
            let (_, response) = try await session.data(for: request)
            #expect((response as? HTTPURLResponse)?.statusCode == 200)
            return traceID
        }
        await Task.detached { runtime.flush() }.value
        let resource = runtime.traces.getActiveResource().attributes
        print(
            "FOLIO_SWIFT_SMOKE trace_id=\(traceID) installation_id=\(resource["folio.installation.id"]!) instance_id=\(resource["service.instance.id"]!)"
        )
        telemetry.stop()
    }

    @Test func installationResourceSurvivesReconfigurationWithoutDeviceIdentifiers() throws {
        let suite = "folio-telemetry-\(UUID())"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = TestInstallationIDStore()
        let identity = InstallationIdentity(defaults: defaults, store: store)
        let installation = try #require(identity.resolve())
        let first = FolioTelemetry.resource(installationID: installation, version: "0.3.4")
        let second = try FolioTelemetry.resource(installationID: #require(identity.resolve()), version: "0.3.5")
        #expect(first.attributes["folio.installation.id"] == second.attributes["folio.installation.id"])
        #expect(first.attributes["service.instance.id"] != first.attributes["folio.installation.id"])
        #expect(first.attributes["service.instance.id"] == second.attributes["service.instance.id"])
        #expect(first.attributes["service.namespace"] == .string("folio"))
        #expect(first.attributes["service.name"] == .string("folio-app"))
        #expect(first.attributes["service.version"] == .string("0.3.4"))
        #expect(first.attributes["os.type"] == .string("darwin"))
        #expect(first.attributes["device.manufacturer"] == .string("Apple"))
        #expect(
            Set(first.attributes.keys) == [
                "service.name", "service.namespace", "service.version", "service.instance.id",
                "folio.installation.id", "os.type", "os.name", "os.version", "device.manufacturer",
                "device.model.identifier",
                "host.arch", "telemetry.sdk.name", "telemetry.sdk.language", "telemetry.sdk.version",
            ]
        )
    }

    @Test func propagationExcludesOtherOriginsAuthenticationAndExports() {
        #expect(
            TelemetryRequestPolicy.allows(
                URL(string: "https://folio.example:443/api/documents?q=secret"), origin: baseURL
            )
        )
        #expect(
            !TelemetryRequestPolicy.allows(URL(string: "https://third-party.example/api/documents"), origin: baseURL)
        )
        #expect(!TelemetryRequestPolicy.allows(URL(string: "http://folio.example/api/documents"), origin: baseURL))
        #expect(
            !TelemetryRequestPolicy.allows(URL(string: "https://folio.example:8443/api/documents"), origin: baseURL)
        )
        #expect(
            !TelemetryRequestPolicy.allows(
                URL(string: "https://folio.example/api/telemetry/v1/traces"), origin: baseURL
            )
        )
        #expect(!TelemetryRequestPolicy.allows(URL(string: "https://folio.example/api/oauth/token"), origin: baseURL))
        #expect(
            !TelemetryRequestPolicy.allows(
                URL(string: "https://folio.example/oauth/authorize?code=secret"), origin: baseURL
            )
        )
        #expect(
            TelemetryRequestPolicy.route(
                URL(
                    string:
                        "https://folio.example/api/documents/3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f/download?token=secret"
                )
            ) == "/api/documents/{id}/download"
        )
    }

    @Test func exportsAllSignalsWithOAuthResourceAndCorrelatedTrace() async throws {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            if request.url?.path == "/api/telemetry/config" {
                return (
                    200,
                    Data(
                        #"{"enabled":true,"endpoint":"/api/telemetry","serviceNamespace":"folio","serviceVersion":"0.3.4"}"#
                            .utf8
                    )
                )
            }
            return (200, Data("{}".utf8))
        }
        let suite = "folio-telemetry-\(UUID())"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let telemetry = FolioTelemetry(
            defaults: defaults, session: URLSession(configuration: config), installationStore: TestInstallationIDStore()
        )
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        let network = URLSession(configuration: config)
        let value = try await telemetry.operation("documents.list") {
            _ = try await network.data(from: URL(string: "https://folio.example/api/documents?q=secret-document-text")!)
            _ = try await network.data(from: URL(string: "https://third-party.example/api/documents")!)
            return 42
        }
        #expect(value == 42)
        let runtime = try #require(telemetry.current)
        await Task.detached { runtime.flush() }.value
        telemetry.stop()
        let exports = capture.requests.filter { $0.0.url?.path.hasPrefix("/api/telemetry/v1/") == true }
        #expect(Set(exports.compactMap { $0.0.url?.lastPathComponent }) == ["traces", "logs", "metrics"])
        let trace = try #require(exports.first { $0.0.url?.lastPathComponent == "traces" })
        let log = try #require(exports.first { $0.0.url?.lastPathComponent == "logs" })
        let traceData = try Opentelemetry_Proto_Collector_Trace_V1_ExportTraceServiceRequest(serializedBytes: trace.1)
        let logData = try Opentelemetry_Proto_Collector_Logs_V1_ExportLogsServiceRequest(serializedBytes: log.1)
        let spans = try #require(traceData.resourceSpans.first?.scopeSpans.first?.spans)
        let operation = try #require(spans.first { $0.name == "documents.list" })
        let logRecords = try #require(logData.resourceLogs.first?.scopeLogs.first?.logRecords)
        #expect(logRecords.contains { $0.traceID == operation.traceID && $0.spanID == operation.spanID })
        let resources = try #require(traceData.resourceSpans.first?.resource.attributes)
        #expect(resources.contains { $0.key == "folio.installation.id" })
        let metric = try #require(exports.first { $0.0.url?.lastPathComponent == "metrics" })
        let metricData = try Opentelemetry_Proto_Collector_Metrics_V1_ExportMetricsServiceRequest(
            serializedBytes: metric.1
        )
        #expect(
            metricData.resourceMetrics.first?.scopeMetrics.first?.metrics.contains { $0.name == "folio.app.operations" }
                == true
        )
        let sameOrigin = try #require(
            capture.requests.first { $0.0.url?.path == "/api/documents" && $0.0.url?.host == "folio.example" }
        )
        let foreign = try #require(capture.requests.first { $0.0.url?.host == "third-party.example" })
        let parent = try #require(sameOrigin.0.value(forHTTPHeaderField: "traceparent"))
        #expect(parent.split(separator: "-").count == 4)
        #expect(parent.contains(operation.traceID.map { String(format: "%02x", $0) }.joined()))
        #expect(foreign.0.value(forHTTPHeaderField: "traceparent") == nil)
        #expect(try !String(decoding: traceData.jsonUTF8Data(), as: UTF8.self).contains("secret-document-text"))
        #expect(trace.0.value(forHTTPHeaderField: "Authorization") == "Bearer first-token")
        #expect(trace.0.value(forHTTPHeaderField: "Content-Type") == "application/x-protobuf")
        #expect(trace.0.value(forHTTPHeaderField: "traceparent") == nil)
    }

    @Test func exportRefreshesOAuthOnceAndStopsAfterSignOut() async throws {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            return (request.value(forHTTPHeaderField: "Authorization") == "Bearer first-token" ? 401 : 200, Data())
        }
        let transport = TelemetryHTTPClient(
            baseURL: baseURL, provider: TelemetryToken(), session: URLSession(configuration: config)
        )
        var request = URLRequest(url: baseURL.appending(path: "api/telemetry/v1/traces"))
        request.httpMethod = "POST"
        request.httpBody = Data([1, 2, 3])
        #expect(try await transport.send(request: request).statusCode == 200)
        #expect(
            capture.requests.map { $0.0.value(forHTTPHeaderField: "Authorization") } == [
                "Bearer first-token", "Bearer refreshed-token",
            ]
        )
        #expect(capture.requests.last?.1 == Data([1, 2, 3]))
        transport.disable()
        await #expect(throws: URLError.self) { try await transport.send(request: request) }
        #expect(capture.requests.count == 2)
    }

    @Test func redirectGuardRefusesForeignOrigins() async throws {
        let session = URLSession(configuration: .ephemeral)
        let task = session.dataTask(with: baseURL.appending(path: "api/documents"))
        let response = try #require(HTTPURLResponse(url: baseURL, statusCode: 302, httpVersion: nil, headerFields: nil))
        let foreign = try URLRequest(url: #require(URL(string: "https://third-party.example/api/documents")))
        let redirected = await withCheckedContinuation { continuation in
            TelemetryRedirectGuard().urlSession(
                session, task: task, willPerformHTTPRedirection: response,
                newRequest: foreign
            ) { continuation.resume(returning: $0) }
        }
        #expect(redirected == nil)
        session.invalidateAndCancel()
    }

    @Test func unavailableKeychainDefersAndAllowsConfigurationRetry() async throws {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            return (200, Data(#"{"enabled":true,"endpoint":"/api/telemetry","serviceNamespace":"folio"}"#.utf8))
        }
        let suite = "folio-telemetry-keychain-test-\(UUID())"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = TestInstallationIDStore()
        store.readUnavailable = true
        let telemetry = FolioTelemetry(
            defaults: defaults, session: URLSession(configuration: config), installationStore: store
        )
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        #expect(await telemetry.operation("documents.list") { 42 } == 42)
        #expect(telemetry.current == nil)
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        #expect(telemetry.current == nil)
        #expect(capture.requests.count == 2)
        #expect(defaults.string(forKey: "folio.telemetry.installationID") == nil)
    }

    @Test(arguments: [200, 503]) func serverDisabledOrUnavailableLeavesOperationsUninstrumented(status: Int)
        async
    {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            return (status, Data(#"{"enabled":false,"endpoint":"/api/telemetry","serviceNamespace":"folio"}"#.utf8))
        }
        let telemetry = FolioTelemetry(
            session: URLSession(configuration: config), installationStore: TestInstallationIDStore()
        )
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        #expect(await telemetry.operation("documents.list") { 42 } == 42)
        #expect(telemetry.current == nil)
        #expect(capture.requests.count == 1)
    }
}

private final class TelemetryURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: ((URLRequest, Data) -> (Int, Data))?
    override class func canInit(with _: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        var body = request.httpBody ?? Data()
        if let stream = request.httpBodyStream {
            stream.open()
            defer { stream.close() }
            var buffer = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable {
                let count = stream.read(&buffer, maxLength: buffer.count)
                if count <= 0 {
                    break
                }
                body.append(buffer, count: count)
            }
        }
        let reply = Self.handler?(request, body) ?? (500, Data())
        let response = HTTPURLResponse(
            url: request.url!, statusCode: reply.0, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: reply.1)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}
