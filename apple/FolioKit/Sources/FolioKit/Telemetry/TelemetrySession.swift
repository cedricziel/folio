import Darwin
import Foundation

/// Why a client session ended, exported as `folio.session.end_reason`.
public enum SessionEndReason: String, Sendable {
    case timeout
    case maxDuration = "max_duration"
    case signOut = "sign_out"
    case serverChange = "server_change"
}

/// A session start or end, for the caller to export as a `session.start` or `session.end` event.
enum SessionBoundary: Equatable, Sendable {
    case start(id: String, previousID: String?)
    case end(id: String, reason: SessionEndReason, at: Date)
}

/// One client session shared by the app, its extensions and intents through a file in the App Group container.
///
/// Every use re-reads the file under an exclusive `flock`, so a rotation or sign-out by one process is seen by
/// the others on their next read, and two processes can't both rotate an expired session. Names follow
/// OpenTelemetry semantic conventions v1.45.0 (`session.*`, Development status).
final class TelemetrySession: @unchecked Sendable {
    static let inactivity: TimeInterval = 30 * 60
    static let maximum: TimeInterval = 4 * 60 * 60

    struct Record: Codable, Equatable {
        var id: String
        var startedAt: Date
        var lastActivityAt: Date
    }

    /// The App Group record in Folio's own processes. Anything else, such as a test runner, keeps the session in
    /// memory and never touches an installed app's state.
    static let shared = TelemetrySession(
        directory: Bundle.main.bundleIdentifier?.hasPrefix("com.cedricziel.folio") == true
            ? FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: SharedConfiguration.appGroup)
            : nil
    )

    private let fileURL: URL?
    private let lockURL: URL?
    private let now: @Sendable () -> Date
    private let lock = NSLock()
    private var memory: Record?

    /// Without a directory the session lives in this process only.
    init(directory: URL?, now: @escaping @Sendable () -> Date = { Date() }) {
        fileURL = directory?.appending(path: "telemetry-session.json")
        lockURL = directory?.appending(path: "telemetry-session.lock")
        self.now = now
    }

    /// The live session ID, without counting as activity.
    func current() -> String? {
        withRecord(write: false) { record, now in
            guard let session = record, Self.expiry(session, at: now) == nil else { return nil }
            return session.id
        }
    }

    /// Records user activity: extends the live session, or ends an expired one and starts the next.
    func touch() -> (id: String, boundaries: [SessionBoundary]) {
        withRecord(write: true) { record, now in
            if let session = record, Self.expiry(session, at: now) == nil {
                record?.lastActivityAt = now
                return (id: session.id, boundaries: [])
            }
            var boundaries: [SessionBoundary] = []
            var previousID: String?
            if let old = record, let reason = Self.expiry(old, at: now) {
                boundaries.append(.end(id: old.id, reason: reason, at: old.lastActivityAt))
                previousID = old.id
            }
            let id = UUID().uuidString.lowercased()
            record = Record(id: id, startedAt: now, lastActivityAt: now)
            boundaries.append(.start(id: id, previousID: previousID))
            return (id: id, boundaries: boundaries)
        }
    }

    /// Ends the session and forgets it, so the next one is not linked to it.
    func end(_ reason: SessionEndReason) -> [SessionBoundary] {
        withRecord(write: true) { record, now in
            let live = record.flatMap { Self.expiry($0, at: now) == nil ? $0 : nil }
            record = nil
            return live.map { [SessionBoundary.end(id: $0.id, reason: reason, at: now)] } ?? []
        }
    }

    /// Why `record` is no longer live at `now`, or nil while it is. A clock set back counts as expired.
    static func expiry(_ record: Record, at now: Date) -> SessionEndReason? {
        if now < record.lastActivityAt || now < record.startedAt { return .timeout }
        if now.timeIntervalSince(record.startedAt) >= maximum { return .maxDuration }
        if now.timeIntervalSince(record.lastActivityAt) > inactivity { return .timeout }
        return nil
    }

    private func withRecord<T>(write: Bool, _ body: (inout Record?, Date) -> T) -> T {
        lock.withLock {
            withFileLock {
                var record = read()
                let original = record
                let result = body(&record, now())
                if write, record != original { save(record) }
                return result
            }
        }
    }

    private func withFileLock<T>(_ body: () -> T) -> T {
        guard let lockURL else { return body() }
        let descriptor = open(lockURL.path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { return body() }
        defer { close(descriptor) }
        while flock(descriptor, LOCK_EX) != 0, errno == EINTR {}
        defer { flock(descriptor, LOCK_UN) }
        return body()
    }

    private func read() -> Record? {
        guard let fileURL else { return memory }
        guard let data = try? Data(contentsOf: fileURL) else { return nil }
        return try? JSONDecoder().decode(Record.self, from: data)
    }

    private func save(_ record: Record?) {
        guard let fileURL else {
            memory = record
            return
        }
        if let record, let data = try? JSONEncoder().encode(record) {
            try? data.write(to: fileURL, options: .atomic)
        } else {
            try? FileManager.default.removeItem(at: fileURL)
        }
    }
}
