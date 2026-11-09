import Foundation
import os

/// Where intents get their server. They run in the app's process, or in a system process that
/// loads the app's code, so the address and sign-in come from the App Group and shared keychain
/// rather than from anything the app holds in memory.
public enum IntentEnvironment {
    public typealias ServiceFactory = @Sendable () throws -> (any FolioService)?

    private static let factory = OSAllocatedUnfairLock<ServiceFactory>(initialState: signedInService)

    /// Swaps the server connection, for tests.
    public static func use(_ make: @escaping ServiceFactory) {
        factory.withLock { $0 = make }
    }

    private static let spotlightIndex = OSAllocatedUnfairLock<SpotlightIndex>(initialState: .shared)

    /// Where intents report documents that are gone, so Spotlight stops offering them.
    public static var spotlight: SpotlightIndex { spotlightIndex.withLock { $0 } }

    /// Swaps the Spotlight index, for tests.
    public static func use(spotlight: SpotlightIndex) {
        spotlightIndex.withLock { $0 = spotlight }
    }

    /// The server intents talk to.
    public static func serverURL() -> URL? {
        SharedConfiguration.serverURL()
    }

    public typealias IntentRecorder = @Sendable (TelemetryIntent) -> Void

    private static let recorder = OSAllocatedUnfairLock<IntentRecorder>(
        initialState: { FolioTelemetry.shared.intent($0) })

    /// Swaps where Shortcut runs are recorded, for tests.
    public static func recordRuns(with record: @escaping IntentRecorder) {
        recorder.withLock { $0 = record }
    }

    /// Records that a Shortcut ran, by name only.
    static func ran(_ intent: TelemetryIntent) {
        recorder.withLock { $0 }(intent)
    }

    /// Records Scan runs too; `ScanDocumentIntent` lives in FolioScanIntent, which cannot see telemetry.
    /// The app calls this at launch, before any intent runs.
    public static func recordScanRuns() {
        ScanIntentEvents.whenRun { ran(.scanDocument) }
    }

    public typealias IntentFlush = @Sendable () async -> Void

    private static let flusher = OSAllocatedUnfairLock<IntentFlush>(
        initialState: { await FolioTelemetry.shared.flush(timeout: .milliseconds(1500)) })

    /// Swaps how a Shortcut's telemetry is sent before the system suspends it, for tests.
    public static func flushRuns(with flush: @escaping IntentFlush) {
        flusher.withLock { $0 = flush }
    }

    public static func catalog() throws -> DocumentCatalog {
        guard let service = try factory.withLock({ $0 })() else { throw ShortcutError.notSignedIn }
        return DocumentCatalog(service: service)
    }

    /// Runs `work` and rewrites whatever it throws, except cancellation, into a message the person can read. Telemetry is sent before
    /// returning, because the system may suspend an intent's process as soon as `perform` ends.
    public static func run<T>(_ work: (DocumentCatalog) async throws -> T) async throws -> T {
        let flush = flusher.withLock { $0 }
        do {
            let result = try await work(catalog())
            await flush()
            return result
        } catch is CancellationError {
            await flush()
            throw CancellationError()
        } catch {
            await flush()
            throw ShortcutError(mapping: error)
        }
    }

    @Sendable private static func signedInService() throws -> (any FolioService)? {
        guard try KeychainCredentialStore().load()?.isSignedIn == true else { return nil }
        return SharedConfiguration.makeService()
    }
}
