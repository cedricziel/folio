import Foundation
import OpenAPIRuntime
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

@Suite(.serialized, .timeLimit(.minutes(1))) struct TelemetryTests {
    private let baseURL = URL(string: "https://folio.example")!

    @Test(.enabled(if: ProcessInfo.processInfo.environment["FOLIO_TELEMETRY_SMOKE_URL"] != nil))
    func realRelaySmoke() async throws {
        let environment = ProcessInfo.processInfo.environment
        let url = try #require(environment["FOLIO_TELEMETRY_SMOKE_URL"].flatMap(URL.init(string:)))
        let provider = SmokeToken(value: environment["FOLIO_TELEMETRY_SMOKE_TOKEN"] ?? "folio-telemetry-smoke")
        let telemetry = FolioTelemetry(installationStore: TestInstallationIDStore())
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
        let store = TestInstallationIDStore()
        let identity = InstallationIdentity(store: store)
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
                "folio.app.component", "folio.installation.id", "os.type", "os.name", "os.version",
                "device.manufacturer",
                "device.model.identifier",
                "host.arch", "telemetry.sdk.name", "telemetry.sdk.language", "telemetry.sdk.version",
            ]
        )
    }

    @Test(arguments: [
        ("com.cedricziel.folio", "app"), ("com.cedricziel.folio.share", "share"),
        ("com.cedricziel.folio.fileprovider", "fileprovider"), ("com.apple.dt.xctest.tool", "app"),
    ])
    func componentNamesTheRecordingProcess(bundleIdentifier: String, component: String) {
        #expect(FolioTelemetry.component(bundleIdentifier: bundleIdentifier) == component)
        let resource = FolioTelemetry.resource(installationID: UUID(), version: "1", component: component)
        #expect(resource.attributes["folio.app.component"] == .string(component))
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
        let telemetry = FolioTelemetry(
            session: URLSession(configuration: config), installationStore: TestInstallationIDStore()
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

    @Test func failedDownloadSpanCarriesStatusAndErrorTypeWhileCancellationIsNotAnError() async throws {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            if request.url?.path == "/api/telemetry/config" {
                return (200, Data(#"{"enabled":true,"endpoint":"/api/telemetry","serviceNamespace":"folio"}"#.utf8))
            }
            return (503, Data())
        }
        let telemetry = FolioTelemetry(
            session: URLSession(configuration: config), installationStore: TestInstallationIDStore()
        )
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        let service = LiveFolioService(
            baseURL: baseURL, tokenProvider: TelemetryToken(), session: URLSession(configuration: config),
            telemetry: telemetry
        )
        await #expect(throws: FolioError.server(status: 503, message: nil)) {
            try await service.downloadFile(for: Document(title: "x", originalFilename: "x.pdf"))
        }
        await #expect(throws: CancellationError.self) {
            try await telemetry.operation("documents.thumbnail") { throw CancellationError() }
        }
        let runtime = try #require(telemetry.current)
        await Task.detached { runtime.flush() }.value
        telemetry.stop()
        let spans = try capture.requests.filter { $0.0.url?.path == "/api/telemetry/v1/traces" }.flatMap {
            try Opentelemetry_Proto_Collector_Trace_V1_ExportTraceServiceRequest(serializedBytes: $0.1)
                .resourceSpans.flatMap { $0.scopeSpans.flatMap(\.spans) }
        }
        func attributes(_ span: Opentelemetry_Proto_Trace_V1_Span) -> [String: String] {
            Dictionary(
                uniqueKeysWithValues: span.attributes.map { attribute in
                    let value = attribute.value
                    return (attribute.key, value.stringValue.isEmpty ? String(value.intValue) : value.stringValue)
                })
        }
        let download = try #require(spans.first { $0.name == "documents.downloadFile" })
        #expect(download.status.code == .error)
        #expect(attributes(download)["error.type"] == "503")
        #expect(attributes(download)["http.response.status_code"] == "503")
        let cancelled = try #require(spans.first { $0.name == "documents.thumbnail" })
        #expect(cancelled.status.code == .unset)
        #expect(attributes(cancelled)["error.type"] == nil)
        #expect(attributes(cancelled)["folio.outcome"] == "cancelled")
    }

    @Test func operationFailuresAreNamedWithoutMessages() {
        #expect(OperationFailure(URLError(.timedOut)) == .init(type: "timeout"))
        #expect(OperationFailure(URLError(.cancelled)).type == OperationFailure.cancelled)
        #expect(OperationFailure(URLError(.networkConnectionLost)) == .init(type: "unreachable"))
        #expect(OperationFailure(FolioError.signInRequired) == .init(type: "401", status: 401))
        #expect(OperationFailure(FolioError.server(status: 500, message: "secret")) == .init(type: "500", status: 500))
        #expect(OperationFailure(FolioError.unreadableResponse) == .init(type: "unreadable_response"))
        #expect(OperationFailure(FolioError.server(status: 500, message: nil), cancelled: true).type == "cancelled")
        #expect(
            OperationFailure(
                ClientError(
                    operationID: "x", operationInput: (), request: nil, requestBody: nil, baseURL: nil,
                    response: nil, responseBody: nil, causeDescription: "", underlyingError: URLError(.timedOut)
                )
            ) == .init(type: "timeout")
        )
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
        let store = TestInstallationIDStore()
        store.readUnavailable = true
        let telemetry = FolioTelemetry(
            session: URLSession(configuration: config), installationStore: store
        )
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        #expect(await telemetry.operation("documents.list") { 42 } == 42)
        #expect(telemetry.current == nil)
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        #expect(telemetry.current == nil)
        #expect(capture.requests.count == 2)
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

    // URLSessionInstrumentation swizzles URLSession once per process, and the first configured instance
    // keeps deciding header injection. This test stops its instance, so it runs after the tests that
    // check traceparent; the suite is serialized and runs in declaration order.
    @Test func refreshKeepsRecordingAndSendsWhatTheReplacedRuntimeQueued() async throws {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            if request.url?.path == "/api/telemetry/config" {
                return (200, Data(#"{"enabled":true,"endpoint":"/api/telemetry","serviceNamespace":"folio"}"#.utf8))
            }
            return (200, Data("{}".utf8))
        }
        let telemetry = FolioTelemetry(
            session: URLSession(configuration: config), installationStore: TestInstallationIDStore()
        )
        defer { telemetry.stop() }
        telemetry.refreshInterval = 0
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        let first = try #require(telemetry.current)
        _ = await telemetry.operation("documents.list") { 42 }

        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        let second = try #require(telemetry.current)
        #expect(second !== first)

        var exported: [String] = []
        // Under the 5-second batch delay, so only the retiring flush can deliver it in time.
        for _ in 0..<20 where exported.isEmpty {
            try await Task.sleep(for: .milliseconds(100))
            exported = try capture.requests.filter { $0.0.url?.lastPathComponent == "traces" }.flatMap {
                try Opentelemetry_Proto_Collector_Trace_V1_ExportTraceServiceRequest(serializedBytes: $0.1)
                    .resourceSpans.flatMap(\.scopeSpans).flatMap(\.spans).map(\.name)
            }
        }
        #expect(exported.contains("documents.list"))
    }

    @Test func refusedRefreshStopsTheRuntime() async throws {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        let status = TelemetryCapture()
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            if request.url?.path == "/api/telemetry/config" {
                let refused = status.requests.isEmpty == false
                return (
                    refused ? 403 : 200,
                    Data(#"{"enabled":true,"endpoint":"/api/telemetry","serviceNamespace":"folio"}"#.utf8)
                )
            }
            return (200, Data("{}".utf8))
        }
        let telemetry = FolioTelemetry(
            session: URLSession(configuration: config), installationStore: TestInstallationIDStore()
        )
        defer { telemetry.stop() }
        telemetry.refreshInterval = 0
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        #expect(telemetry.current != nil)

        status.append(URLRequest(url: baseURL), Data())
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())

        #expect(telemetry.current == nil)
    }

    @Test func failedRefreshKeepsTheRuntimeAndRetries() async throws {
        let capture = TelemetryCapture()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [TelemetryURLProtocol.self]
        let outage = TelemetryCapture()
        TelemetryURLProtocol.handler = { request, body in
            capture.append(request, body)
            if request.url?.path == "/api/telemetry/config" {
                return (
                    outage.requests.isEmpty ? 200 : 503,
                    Data(#"{"enabled":true,"endpoint":"/api/telemetry","serviceNamespace":"folio"}"#.utf8)
                )
            }
            return (200, Data("{}".utf8))
        }
        let telemetry = FolioTelemetry(
            session: URLSession(configuration: config), installationStore: TestInstallationIDStore()
        )
        defer { telemetry.stop() }
        telemetry.refreshInterval = 0
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        let running = try #require(telemetry.current)

        outage.append(URLRequest(url: baseURL), Data())
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        #expect(telemetry.current === running)

        // The failed attempt does not count as a configuration: even with the normal interval, the next call
        // asks again instead of waiting five minutes.
        telemetry.refreshInterval = 300
        await telemetry.configure(baseURL: baseURL, tokenProvider: TelemetryToken())
        let configRequests = capture.requests.filter { $0.0.url?.path == "/api/telemetry/config" }
        #expect(configRequests.count == 3)
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
