import AppIntents
import Foundation
import os

/// Opens the app's scanner. It lives apart from FolioKit's other intents so the widget extension can offer it
/// without registering the data intents and entity queries in its own process.
public struct ScanDocumentIntent: AppIntent {
    public static let title: LocalizedStringResource = "Scan a document"
    public static let description = IntentDescription(
        "Opens Folio's scanner so you can photograph a paper document and file it."
    )
    public static let openAppWhenRun = true

    public init() {}

    @MainActor
    public func perform() async throws -> some IntentResult {
        ScanIntentEvents.ran()
        IntentRouter.shared.route(.scan)
        return .result()
    }
}

/// Lets FolioKit record a scan run without this module depending on its telemetry.
public enum ScanIntentEvents {
    private static let onRun = OSAllocatedUnfairLock<@Sendable () -> Void>(initialState: {})

    public static func whenRun(_ action: @escaping @Sendable () -> Void) {
        onRun.withLock { $0 = action }
    }

    static func ran() {
        onRun.withLock { $0 }()
    }
}

public enum FolioScanIntentPackage: AppIntentsPackage {}
