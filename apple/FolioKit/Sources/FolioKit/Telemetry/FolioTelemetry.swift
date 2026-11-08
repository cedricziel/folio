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
    private var lastFlush = ContinuousClock.Instant.now - .seconds(60)
    private var flushPending = false
    private var instrumentation: URLSessionInstrumentation?
    private let installationIdentity: InstallationIdentity
    private let session: URLSession
    // Session and screen belong to the process, not to a runtime that each reconfigure replaces.
    private let sessions: TelemetrySession
    private var currentScreen: TelemetryScreen?
    private var lastScreenAt = ContinuousClock.Instant.now - .seconds(60)
    private var pendingBoundaries: [SessionBoundary] = []
    private var limiter = UIEventLimiter()

    public convenience init(
        session: URLSession? = nil,
        installationStore: any InstallationIDStore = KeychainInstallationIDStore()
    ) {
        self.init(session: session, installationStore: installationStore, sessions: .shared)
    }

    init(session: URLSession?, installationStore: any InstallationIDStore, sessions: TelemetrySession) {
        installationIdentity = InstallationIdentity(store: installationStore)
        self.session =
            session ?? URLSession(configuration: .ephemeral, delegate: TelemetryRedirectGuard(), delegateQueue: nil)
        self.sessions = sessions
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

    /// How long accepted settings are reused before they are fetched again.
    var refreshInterval: TimeInterval = 300

    /// A refresh for the same server keeps the current runtime recording until its replacement is accepted.
    private func reserve(baseURL: URL, tokenProvider: any BearerTokenProvider) -> UUID? {
        lock.withLock {
            guard configuredURL != baseURL || Date().timeIntervalSince(lastConfiguration) >= refreshInterval else {
                return nil
            }
            if let previousURL = configuredURL, previousURL != baseURL {
                // A new server starts a new session; the old runtime's queue is discarded below.
                _ = sessions.end(.serverChange)
                pendingBoundaries = []
            }
            if configuredURL != baseURL {
                // Signals recorded for another server must not be sent once its credentials are gone.
                runtime?.transport.disable()
                let previous = runtime
                runtime = nil
                if let previous { Task.detached { previous.shutdown() } }
            }
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
                let disabled = lock.withLock { () -> TelemetryRuntime? in
                    guard generation == ticket else { return nil }
                    defer { runtime = nil }
                    return runtime
                }
                if let disabled { Task.detached { disabled.shutdown() } }
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
            var retired: TelemetryRuntime?
            let accepted = lock.withLock {
                guard generation == ticket else { return false }
                retired = runtime
                runtime = next
                if instrumentation == nil {
                    instrumentation = URLSessionInstrumentation(
                        configuration: .init(
                            shouldRecordPayload: { _ in false },
                            shouldInstrument: { [weak self] in self?.allows($0) ?? false },
                            spanCustomization: { [weak self] request, builder in
                                for (key, value) in self?.stamp() ?? [:] {
                                    builder.setAttribute(key: key, value: value)
                                }
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
            if let retired { Task.detached { retired.retire() } }
            guard accepted else {
                next.shutdown()
                return
            }
            next.event("folio.telemetry.configured")
            // What happened before export was possible: session boundaries, and the screen on show.
            let (queued, screen) = lock.withLock { () -> ([SessionBoundary], TelemetryScreen?) in
                defer { pendingBoundaries = [] }
                return (pendingBoundaries, currentScreen)
            }
            for boundary in queued { next.boundary(boundary) }
            if retired == nil, let screen {
                next.uiEvent("folio.screen.view", attributes: stamp(["app.screen.name": .string(screen.rawValue)]))
            }
        } catch {
            // Optional observability must never block sign-in or document access. A server that refuses the
            // settings request (401, 403, no token) stops collection. Any other failure, such as a 5xx while the
            // server restarts or no network, keeps the current runtime and lets the next call retry.
            let refused =
                (error as? TelemetryHTTPStatus).map { [401, 403].contains($0.code) } ?? false
                || (error as? URLError)?.code == .userAuthenticationRequired
            let stale = lock.withLock { () -> TelemetryRuntime? in
                guard generation == ticket else { return nil }
                guard refused else {
                    lastConfiguration = .distantPast
                    return nil
                }
                defer { runtime = nil }
                return runtime
            }
            if let stale { Task.detached { stale.shutdown() } }
        }
    }

    /// Stops collection immediately and discards queued exports on sign-out.
    public func stop() {
        _ = sessions.end(.signOut)
        let previous = lock.withLock {
            pendingBoundaries = []
            currentScreen = nil
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

    /// Ends the session and sends its `session.end` while credentials still work. Call before signing out.
    public func endSession(_ reason: SessionEndReason) async {
        let boundaries = sessions.end(reason)
        lock.withLock {
            pendingBoundaries = []
            currentScreen = nil
        }
        guard let runtime = current else { return }
        for boundary in boundaries { runtime.boundary(boundary) }
        await flush(timeout: .milliseconds(1500))
    }

    /// Records that the person is using Folio: extends the shared session, or starts the next one.
    public func activity() {
        record(sessions.touch().boundaries)
    }

    /// Records the screen on show. The same screen again within a second is not reported twice.
    public func screen(_ screen: TelemetryScreen) {
        let boundaries = sessions.touch().boundaries
        let repeated = lock.withLock { () -> Bool in
            let now = ContinuousClock.now
            defer {
                currentScreen = screen
                lastScreenAt = now
            }
            return currentScreen == screen && now - lastScreenAt < .seconds(1)
        }
        record(
            boundaries,
            event: repeated ? nil : ("folio.screen.view", ["app.screen.name": .string(screen.rawValue)]))
    }

    /// Records a deliberate user action by name. Actions before telemetry is configured are dropped.
    public func action(_ action: TelemetryAction, upload source: UploadSource? = nil) {
        var attributes: [String: AttributeValue] = ["folio.action.name": .string(action.rawValue)]
        if let source { attributes["folio.upload.source"] = .string(source.rawValue) }
        record(sessions.touch().boundaries, event: ("folio.action", attributes))
    }

    /// Records that a Shortcut ran.
    public func intent(_ intent: TelemetryIntent) {
        record(
            sessions.touch().boundaries,
            event: (
                "folio.action",
                [
                    "folio.action.name": .string(TelemetryAction.intentRun.rawValue),
                    "folio.intent.name": .string(intent.rawValue),
                ]
            ))
    }

    /// Sends session boundaries and a UI event now, or keeps the latest boundaries until a runtime is ready.
    private func record(_ boundaries: [SessionBoundary], event: (String, [String: AttributeValue])? = nil) {
        let target = lock.withLock { () -> (TelemetryRuntime, Bool, Int)? in
            guard let runtime else {
                pendingBoundaries.append(contentsOf: boundaries)
                if pendingBoundaries.count > 8 { pendingBoundaries.removeFirst(pendingBoundaries.count - 8) }
                return nil
            }
            let allowed = event == nil || limiter.allow(at: .now)
            return (runtime, allowed, limiter.takeDropped())
        }
        guard let target else { return }
        let (runtime, allowed, dropped) = target
        for boundary in boundaries { runtime.boundary(boundary) }
        if let event, allowed { runtime.uiEvent(event.0, attributes: stamp(event.1)) }
        if dropped > 0 { runtime.dropped(dropped) }
    }

    /// The live session and current screen, under `attributes`, for spans and log records. Never metrics.
    func stamp(_ attributes: [String: AttributeValue] = [:]) -> [String: AttributeValue] {
        var stamped: [String: AttributeValue] = [:]
        if let id = sessions.current() { stamped["session.id"] = .string(id) }
        if let screen = lock.withLock({ currentScreen }) { stamped["app.screen.name"] = .string(screen.rawValue) }
        return stamped.merging(attributes) { _, explicit in explicit }
    }

    /// Records an app lifecycle transition without view, account, or document details.
    public func lifecycle(_ phase: String) {
        guard ["active", "inactive", "background"].contains(phase) else { return }
        if phase == "active" { activity() }
        current?.event("app.lifecycle.\(phase)")
        if phase == "active",
            let target = lock.withLock({ configuredURL.flatMap { url in configuredProvider.map { (url, $0) } } })
        {
            start(baseURL: target.0, tokenProvider: target.1)
        }
        if phase == "background" { flush() }
    }

    /// Runs a named operation; only the caller's fixed operation name, `attributes` and outcome are exported.
    /// `attributes` go on the span and its completion log, never on metrics.
    public func operation<T>(
        _ name: String, attributes: [String: AttributeValue] = [:], _ work: () async throws -> T
    ) async rethrows -> T {
        guard let runtime = current else { return try await work() }
        let start = ContinuousClock.now
        let context = stamp(attributes)
        let builder = context.reduce(runtime.tracer.spanBuilder(spanName: name)) {
            $0.setAttribute(key: $1.key, value: $1.value)
        }
        return try await builder.withActiveSpan { span in
            do {
                let result = try await work()
                runtime.completed(
                    name, outcome: "success", duration: start.duration(to: .now), span: span, context: context)
                return result
            } catch {
                let failure = OperationFailure(error, cancelled: Task.isCancelled)
                if let status = failure.status {
                    span.setAttribute(key: "http.response.status_code", value: status)
                }
                if failure.isCancellation {
                    span.setAttribute(key: "folio.outcome", value: "cancelled")
                    runtime.completed(
                        name, outcome: "cancelled", duration: start.duration(to: .now), span: span,
                        context: context)
                } else {
                    span.setAttribute(key: "error.type", value: failure.type)
                    span.status = .error(description: failure.type)
                    runtime.completed(
                        name, outcome: "error", duration: start.duration(to: .now), span: span, context: context)
                }
                throw error
            }
        }
    }

    /// Flushes pending signals on a worker task, without delaying the caller. Calls made while a flush is
    /// pending share it, and flushes start at most once per second, so a burst of operations sends one export.
    public func flush() {
        guard
            let plan = lock.withLock({ () -> (TelemetryRuntime, Duration)? in
                guard let active = self.runtime, !flushPending else { return nil }
                flushPending = true
                let elapsed = ContinuousClock.now - lastFlush
                return (active, elapsed >= .seconds(1) ? .zero : .seconds(1) - elapsed)
            })
        else { return }
        let (target, delay) = plan
        Task.detached { [self] in
            if delay > .zero { try? await Task.sleep(for: delay) }
            lock.withLock {
                flushPending = false
                lastFlush = .now
            }
            target.flush()
        }
    }

    /// Flushes and waits at most `timeout`, for a process the system is about to suspend.
    public func flush(timeout: Duration) async {
        guard let runtime = current else { return }
        // Whichever finishes first resumes the caller; a slow export keeps running on its own.
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            let once = ResumeOnce(continuation)
            Task.detached {
                runtime.flush()
                once.resume()
            }
            Task.detached {
                try? await Task.sleep(for: timeout)
                once.resume()
            }
        }
    }

    var current: TelemetryRuntime? { lock.withLock { runtime } }

    private func allows(_ request: URLRequest) -> Bool {
        guard let runtime = current else { return false }
        return TelemetryRequestPolicy.allows(request.url, origin: runtime.baseURL)
    }

    /// Which Folio process records: the app, or the extension named by its bundle identifier suffix.
    static func component(bundleIdentifier: String?) -> String {
        let suffix = bundleIdentifier?.split(separator: ".").last.map(String.init) ?? ""
        return ["share", "fileprovider"].contains(suffix) ? suffix : "app"
    }

    static func resource(
        installationID: UUID, version: String? = nil,
        component: String = FolioTelemetry.component(bundleIdentifier: Bundle.main.bundleIdentifier)
    ) -> Resource {
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
                "service.instance.id": .string(processID), "folio.app.component": .string(component),
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

/// A telemetry request the server answered with a non-2xx status.
struct TelemetryHTTPStatus: Error, Equatable {
    let code: Int
}

/// Resumes a continuation once, from whichever caller gets there first.
private final class ResumeOnce: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Void, Never>?

    init(_ continuation: CheckedContinuation<Void, Never>) {
        self.continuation = continuation
    }

    func resume() {
        let pending = lock.withLock { () -> CheckedContinuation<Void, Never>? in
            defer { continuation = nil }
            return continuation
        }
        pending?.resume()
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
            throw TelemetryHTTPStatus(code: (response as? HTTPURLResponse)?.statusCode ?? 0)
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
    private var droppedEvents: LongCounter

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
            .registerView(
                selector: InstrumentSelector.builder().setInstrument(name: "folio[.]telemetry[.].*").build(),
                view: View.builder().build()
            )
            .registerMetricReader(reader: reader).build()
        let meter = metrics.get(name: "folio-app")
        operations = meter.counterBuilder(name: "folio.app.operations").build()
        duration = meter.histogramBuilder(name: "folio.app.operation.duration").setUnit("s").build()
        droppedEvents = meter.counterBuilder(name: "folio.telemetry.ui_events.dropped").build()
    }

    /// `context` (session, screen, browse details) goes on the log only; metrics keep operation and outcome.
    func completed(
        _ name: String, outcome: String, duration elapsed: Duration, span: any SpanBase,
        context: [String: AttributeValue] = [:]
    ) {
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
            .setSeverity(outcome == "success" ? .info : .error)
            .setAttributes(context.merging(attributes) { _, operation in operation })
            .setSpanContext(span.context).emit()
    }

    /// A log-only event: no span and no metric point.
    func uiEvent(_ name: String, attributes: [String: AttributeValue], at timestamp: Date = Date()) {
        logs.get(instrumentationScopeName: "folio-app").logRecordBuilder().setTimestamp(timestamp)
            .setEventName(name).setSeverity(.info).setAttributes(attributes).emit()
    }

    func boundary(_ boundary: SessionBoundary) {
        switch boundary {
        case .start(let id, let previousID):
            var attributes: [String: AttributeValue] = ["session.id": .string(id)]
            if let previousID { attributes["session.previous_id"] = .string(previousID) }
            uiEvent("session.start", attributes: attributes)
        case .end(let id, let reason, let at):
            uiEvent(
                "session.end",
                attributes: ["session.id": .string(id), "folio.session.end_reason": .string(reason.rawValue)],
                at: at)
        }
    }

    func dropped(_ count: Int) {
        measurementLock.withLock { droppedEvents.add(value: count, attributes: [:]) }
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

    /// Sends what is still queued, then stops; for a runtime replaced while its server stays the same.
    func retire() {
        flush()
        shutdown()
    }

    func shutdown() {
        transport.disable()
        traces.shutdown()
        _ = logProcessor.shutdown(explicitTimeout: 1)
        _ = metrics.shutdown()
    }
}
