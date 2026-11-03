import Darwin
import Foundation
import OpenAPIRuntime
import OpenTelemetryApi
import OpenTelemetryConcurrency
import OpenTelemetryProtocolExporterCommon
import OpenTelemetryProtocolExporterHttp
import OpenTelemetrySdk
import URLSessionInstrumentation

/// Collects app operations only after the signed-in server enables its telemetry relay.
public final class FolioTelemetry: @unchecked Sendable {
    public static let shared = FolioTelemetry()
    private static let processID = UUID().uuidString.lowercased()
    private static let contextInstalled: Void = {
        OpenTelemetryConcurrency.OpenTelemetry.registerDefaultConcurrencyContextManager()
    }()

    private let lock = NSLock()
    private var runtime: TelemetryRuntime?
    private var generation = UUID()
    private var configuredURL: URL?
    private var configuredProvider: (any BearerTokenProvider)?
    private var lastConfiguration = Date.distantPast
    private var instrumentation: URLSessionInstrumentation?
    private let installationIdentity: InstallationIdentity
    private let session: URLSession

    public init(
        session: URLSession? = nil,
        installationStore: any InstallationIDStore = KeychainInstallationIDStore()
    ) {
        installationIdentity = InstallationIdentity(store: installationStore)
        self.session =
            session ?? URLSession(configuration: .ephemeral, delegate: TelemetryRedirectGuard(), delegateQueue: nil)
        _ = Self.contextInstalled
    }

    /// Starts configuration without delaying app or extension startup.
    public func start(baseURL: URL, tokenProvider: any BearerTokenProvider) {
        guard let ticket = reserve(baseURL: baseURL, tokenProvider: tokenProvider) else { return }
        Task { await configure(baseURL: baseURL, tokenProvider: tokenProvider, ticket: ticket) }
    }

    /// Fetches the authenticated relay settings. Failures leave telemetry disabled.
    public func configure(baseURL: URL, tokenProvider: any BearerTokenProvider) async {
        guard let ticket = reserve(baseURL: baseURL, tokenProvider: tokenProvider) else { return }
        await configure(baseURL: baseURL, tokenProvider: tokenProvider, ticket: ticket)
    }

    private func reserve(baseURL: URL, tokenProvider: any BearerTokenProvider) -> UUID? {
        lock.withLock {
            guard configuredURL != baseURL || Date().timeIntervalSince(lastConfiguration) >= 300 else { return nil }
            runtime?.transport.disable()
            let previous = runtime
            runtime = nil
            if let previous { Task.detached { previous.shutdown() } }
            configuredURL = baseURL
            configuredProvider = tokenProvider
            lastConfiguration = Date()
            generation = UUID()
            return generation
        }
    }

    private func configure(baseURL: URL, tokenProvider: any BearerTokenProvider, ticket: UUID) async {
        guard lock.withLock({ generation == ticket }) else { return }
        let transport = TelemetryHTTPClient(baseURL: baseURL, provider: tokenProvider, session: session)
        var request = URLRequest(url: baseURL.appending(path: "api/telemetry/config"))
        request.timeoutInterval = 5
        do {
            let (data, _) = try await transport.data(for: request)
            let config = try JSONDecoder().decode(TelemetryConfiguration.self, from: data)
            guard config.enabled, config.serviceNamespace == "folio", config.endpoint == "/api/telemetry" else {
                return
            }
            guard let installation = installationIdentity.resolve() else {
                lock.withLock {
                    if generation == ticket { lastConfiguration = .distantPast }
                }
                return
            }
            let resource = Self.resource(installationID: installation)
            let next = TelemetryRuntime(baseURL: baseURL, transport: transport, resource: resource)
            let accepted = lock.withLock {
                guard generation == ticket else { return false }
                runtime = next
                if instrumentation == nil {
                    instrumentation = URLSessionInstrumentation(
                        configuration: .init(
                            shouldRecordPayload: { _ in false },
                            shouldInstrument: { [weak self] in self?.allows($0) ?? false },
                            spanCustomization: { request, builder in
                                let route = TelemetryRequestPolicy.route(request.url)
                                builder.setAttribute(
                                    key: "url.full", value: TelemetryRequestPolicy.redactedURL(request.url))
                                builder.setAttribute(key: "url.path", value: route)
                                if let parent = OpenTelemetryApi.OpenTelemetry.instance.contextProvider.activeSpan {
                                    builder.setParent(parent)
                                }
                            },
                            shouldInjectTracingHeaders: { [weak self] in self?.allows($0) ?? false },
                            tracer: next.tracer, semanticConvention: .stable
                        ))
                } else {
                    var config = instrumentation!.configuration
                    config.tracer = next.tracer
                    instrumentation!.configuration = config
                }
                return true
            }
            if accepted { next.event("app.session.start") } else { next.shutdown() }
        } catch {
            // Optional observability must never block sign-in or document access.
        }
    }

    /// Stops collection immediately and discards queued exports on sign-out.
    public func stop() {
        let previous = lock.withLock {
            generation = UUID()
            configuredURL = nil
            configuredProvider = nil
            let previous = runtime
            previous?.transport.disable()
            runtime = nil
            return previous
        }
        if let previous { Task.detached { previous.shutdown() } }
    }

    /// Records an app lifecycle transition without view, account, or document details.
    public func lifecycle(_ phase: String) {
        guard ["active", "inactive", "background"].contains(phase) else { return }
        current?.event("app.lifecycle.\(phase)")
        if phase == "active",
            let target = lock.withLock({ configuredURL.flatMap { url in configuredProvider.map { (url, $0) } } })
        {
            start(baseURL: target.0, tokenProvider: target.1)
        }
        if phase == "background" { flush() }
    }

    /// Runs a named operation; only the caller's fixed operation name and outcome are exported.
    public func operation<T>(_ name: String, _ work: () async throws -> T) async rethrows -> T {
        guard let runtime = current else { return try await work() }
        let start = ContinuousClock.now
        return try await runtime.tracer.spanBuilder(spanName: name).withActiveSpan { span in
            do {
                let result = try await work()
                runtime.completed(name, outcome: "success", duration: start.duration(to: .now), span: span)
                return result
            } catch {
                let failure = OperationFailure(error, cancelled: Task.isCancelled)
                if let status = failure.status {
                    span.setAttribute(key: "http.response.status_code", value: status)
                }
                if failure.isCancellation {
                    span.setAttribute(key: "folio.outcome", value: "cancelled")
                    runtime.completed(name, outcome: "cancelled", duration: start.duration(to: .now), span: span)
                } else {
                    span.setAttribute(key: "error.type", value: failure.type)
                    span.status = .error(description: failure.type)
                    runtime.completed(name, outcome: "error", duration: start.duration(to: .now), span: span)
                }
                throw error
            }
        }
    }

    /// Flushes pending signals on a worker task, without delaying the caller.
    public func flush() {
        if let runtime = current { Task.detached { runtime.flush() } }
    }

    var current: TelemetryRuntime? { lock.withLock { runtime } }

    private func allows(_ request: URLRequest) -> Bool {
        guard let runtime = current else { return false }
        return TelemetryRequestPolicy.allows(request.url, origin: runtime.baseURL)
    }

    static func resource(installationID: UUID, version: String? = nil) -> Resource {
        var system = utsname()
        uname(&system)
        var model = withUnsafeBytes(of: &system.machine) { bytes in
            String(decoding: bytes.prefix { $0 != 0 }, as: UTF8.self)
        }
        #if os(macOS)
            var modelSize = 0
            if sysctlbyname("hw.model", nil, &modelSize, nil, 0) == 0, modelSize > 0 {
                var hardwareModel = [UInt8](repeating: 0, count: modelSize)
                if sysctlbyname("hw.model", &hardwareModel, &modelSize, nil, 0) == 0 {
                    model = String(decoding: hardwareModel.prefix { $0 != 0 }, as: UTF8.self)
                }
            }
        #endif
        #if os(iOS)
            let os = "iOS"
        #else
            let os = "macOS"
        #endif
        #if arch(arm64)
            let architecture = "arm64"
        #elseif arch(x86_64)
            let architecture = "amd64"
        #else
            let architecture = "unknown"
        #endif
        return Resource().merging(
            other: Resource(attributes: [
                "service.name": .string("folio-app"), "service.namespace": .string("folio"),
                "service.version": .string(
                    version ?? Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
                        ?? "unknown"),
                "service.instance.id": .string(processID),
                "folio.installation.id": .string(installationID.uuidString.lowercased()),
                "os.type": .string("darwin"), "os.name": .string(os),
                "os.version": .string(ProcessInfo.processInfo.operatingSystemVersionString),
                "device.manufacturer": .string("Apple"), "device.model.identifier": .string(model),
                "host.arch": .string(architecture),
            ]))
    }
}

/// Names why an operation failed, without exporting messages that may carry document details.
struct OperationFailure: Equatable {
    static let cancelled = "cancelled"

    let type: String
    let status: Int?

    var isCancellation: Bool { type == Self.cancelled }

    init(type: String, status: Int? = nil) {
        self.type = type
        self.status = status
    }

    init(_ error: any Error, cancelled: Bool = false) {
        if cancelled {
            self.init(type: Self.cancelled)
            return
        }
        switch error {
        case is CancellationError:
            self.init(type: Self.cancelled)
        case let error as ClientError:
            let inner = OperationFailure(error.underlyingError)
            self.init(type: inner.type, status: inner.status ?? error.response.map { $0.status.code })
        case let error as URLError:
            self.init(type: Self.name(error.code))
        case FolioError.server(let status, _):
            self.init(type: String(status), status: status)
        case FolioError.signInRequired:
            self.init(type: "401", status: 401)
        case FolioError.unreachable:
            self.init(type: "unreachable")
        case FolioError.unreadableResponse:
            self.init(type: "unreadable_response")
        default:
            self.init(type: "_OTHER")
        }
    }

    private static func name(_ code: URLError.Code) -> String {
        switch code {
        case .cancelled: cancelled
        case .timedOut: "timeout"
        case .notConnectedToInternet, .networkConnectionLost, .cannotConnectToHost, .cannotFindHost,
            .dnsLookupFailed:
            "unreachable"
        case .secureConnectionFailed, .serverCertificateUntrusted, .serverCertificateHasBadDate,
            .serverCertificateNotYetValid, .serverCertificateHasUnknownRoot:
            "tls"
        default: "url_error_\(code.rawValue)"
        }
    }
}

struct TelemetryConfiguration: Decodable {
    let enabled: Bool
    let endpoint: String
    let serviceNamespace: String
}

struct TelemetryRequestPolicy {
    static func sameOrigin(_ url: URL, _ origin: URL) -> Bool {
        func port(_ value: URL) -> Int? { value.port ?? (value.scheme?.lowercased() == "https" ? 443 : 80) }
        return url.scheme?.lowercased() == origin.scheme?.lowercased()
            && url.host?.lowercased() == origin.host?.lowercased() && port(url) == port(origin)
            && url.user == nil && url.password == nil
    }

    static func allows(_ url: URL?, origin: URL) -> Bool {
        guard let url, sameOrigin(url, origin) else { return false }
        return url.path.hasPrefix("/api/") && !url.path.hasPrefix("/api/telemetry")
            && !url.path.hasPrefix("/api/oauth") && !url.path.hasPrefix("/api/auth")
    }

    static func redactedURL(_ url: URL?) -> String {
        guard let url, var components = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return "/api" }
        components.user = nil
        components.password = nil
        components.query = nil
        components.fragment = nil
        components.path = route(url)
        return components.url?.absoluteString ?? "/api"
    }

    static func route(_ url: URL?) -> String {
        guard let url else { return "/api" }
        let parts = url.path.split(separator: "/").map { part in
            UUID(uuidString: String(part)) == nil ? String(part) : "{id}"
        }
        return "/" + parts.joined(separator: "/")
    }
}

final class TelemetryRedirectGuard: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        guard let origin = task.originalRequest?.url, let target = request.url,
            TelemetryRequestPolicy.sameOrigin(target, origin)
        else { completionHandler(nil); return }
        completionHandler(request)
    }
}

final class TelemetryHTTPClient: HTTPClient, @unchecked Sendable {
    let baseURL: URL
    private let provider: any BearerTokenProvider
    private let session: URLSession
    private let lock = NSLock()
    private var active = true

    init(baseURL: URL, provider: any BearerTokenProvider, session: URLSession) {
        self.baseURL = baseURL
        self.provider = provider
        self.session = session
    }

    func disable() { lock.withLock { active = false } }

    func data(for request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        guard let url = request.url, TelemetryRequestPolicy.sameOrigin(url, baseURL),
            url.path.hasPrefix("/api/telemetry"), lock.withLock({ active }),
            let token = try await provider.token()
        else { throw URLError(.userAuthenticationRequired) }
        func signed(_ token: String) -> URLRequest {
            var signed = request
            signed.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            signed.setValue(nil, forHTTPHeaderField: "traceparent")
            signed.setValue(nil, forHTTPHeaderField: "tracestate")
            signed.setValue(nil, forHTTPHeaderField: "baggage")
            return signed
        }
        guard lock.withLock({ active }) else { throw URLError(.cancelled) }
        var (data, response) = try await session.data(for: signed(token))
        if (response as? HTTPURLResponse)?.statusCode == 401,
            let fresh = try await provider.refresh(rejecting: token), lock.withLock({ active })
        {
            (data, response) = try await session.data(for: signed(fresh))
        }
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw URLError(.badServerResponse)
        }
        return (data, http)
    }

    func send(request: URLRequest) async throws -> HTTPURLResponse { try await data(for: request).1 }

    func send(request: URLRequest, completion: @escaping (Result<HTTPURLResponse, Error>) -> Void) {
        nonisolated(unsafe) let completion = completion
        Task {
            do { completion(.success(try await send(request: request))) } catch { completion(.failure(error)) }
        }
    }
}

final class TelemetryRuntime: @unchecked Sendable {
    let baseURL: URL
    let transport: TelemetryHTTPClient
    let tracer: Tracer
    let traces: TracerProviderSdk
    let logs: LoggerProviderSdk
    let metrics: MeterProviderSdk
    private let logProcessor: BatchLogRecordProcessor
    private let measurementLock = NSLock()
    private var operations: LongCounter
    private var duration: DoubleHistogram

    init(baseURL: URL, transport: TelemetryHTTPClient, resource: Resource) {
        self.baseURL = baseURL
        self.transport = transport
        let config = OtlpConfiguration(timeout: 5, compression: .none, exportAsJson: false)
        let endpoint = baseURL.appending(path: "api/telemetry/v1")
        let traceExporter = OtlpHttpTraceExporter(
            endpoint: endpoint.appending(path: "traces"), config: config,
            httpClient: transport, envVarHeaders: [], requeueOnFailure: false)
        traces = TracerProviderBuilder().with(resource: resource).add(
            spanProcessor: BatchSpanProcessor(
                spanExporter: traceExporter, scheduleDelay: 5, exportTimeout: 5, maxQueueSize: 256,
                maxExportBatchSize: 64
            )
        ).build()
        tracer = traces.get(instrumentationName: "folio-app")
        let logExporter = OtlpHttpLogExporter(
            endpoint: endpoint.appending(path: "logs"), config: config,
            httpClient: transport, envVarHeaders: [], requeueOnFailure: false)
        logProcessor = BatchLogRecordProcessor(
            logRecordExporter: logExporter,
            scheduleDelay: 5, exportTimeout: 5, maxQueueSize: 256, maxExportBatchSize: 64)
        logs = LoggerProviderBuilder().with(resource: resource).with(processors: [logProcessor]).build()
        let metricExporter = OtlpHttpMetricExporter(
            endpoint: endpoint.appending(path: "metrics"), config: config,
            httpClient: transport, envVarHeaders: [], requeueOnFailure: false)
        let reader = PeriodicMetricReaderBuilder(exporter: metricExporter).setInterval(timeInterval: 30)
            .setExportTimeout(timeInterval: 5).build()
        metrics = MeterProviderSdk.builder().setResource(resource: resource)
            .registerView(
                selector: InstrumentSelector.builder().setInstrument(name: "folio[.]app[.].*").build(),
                view: View.builder().build()
            )
            .registerMetricReader(reader: reader).build()
        let meter = metrics.get(name: "folio-app")
        operations = meter.counterBuilder(name: "folio.app.operations").build()
        duration = meter.histogramBuilder(name: "folio.app.operation.duration").setUnit("s").build()
    }

    func completed(_ name: String, outcome: String, duration elapsed: Duration, span: any SpanBase) {
        let attributes: [String: AttributeValue] = [
            "folio.operation": .string(name), "folio.outcome": .string(outcome),
        ]
        let seconds = Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18
        measurementLock.withLock {
            operations.add(value: 1, attributes: attributes)
            duration.record(value: seconds, attributes: attributes)
        }
        logs.get(instrumentationScopeName: "folio-app").logRecordBuilder().setTimestamp(Date())
            .setEventName("folio.operation.completed").setBody(.string(name))
            .setSeverity(outcome == "success" ? .info : .error).setAttributes(attributes)
            .setSpanContext(span.context).emit()
    }

    func event(_ name: String) {
        let span = tracer.spanBuilder(spanName: name).startSpan()
        completed(name, outcome: "success", duration: .zero, span: span)
        span.end()
    }

    func flush() {
        traces.forceFlush(timeout: 5)
        logProcessor.forceFlush(timeout: 5)
        _ = metrics.forceFlush()
    }

    func shutdown() {
        transport.disable()
        traces.shutdown()
        _ = logProcessor.shutdown(explicitTimeout: 1)
        _ = metrics.shutdown()
    }
}
