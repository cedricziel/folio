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
/// the others on their next read, and two processes can't both rotate an expired session. Only the app marks the
/// record signed in; sign-out clears that and the session together, so an extension still running afterwards
/// can't create a session that the next account would adopt. Names follow OpenTelemetry semantic conventions
/// v1.45.0 (`session.*`, Development status).
final class TelemetrySession: @unchecked Sendable {
    static let inactivity: TimeInterval = 30 * 60
    static let maximum: TimeInterval = 4 * 60 * 60

    struct State: Codable, Equatable {
        var signedIn = false
        var session: Record?
    }

    struct Record: Codable, Equatable {
        var id: String
        var startedAt: Date
        var lastActivityAt: Date
    }

    /// The App Group record in Folio's own processes. Anything else, such as a test runner, keeps the session in
    /// memory and never touches an installed app's state.
    static let shared = TelemetrySession(directory: sharedDirectory)

    /// The App Group container in Folio's own processes, nil anywhere else.
    static var sharedDirectory: URL? {
        Bundle.main.bundleIdentifier?.hasPrefix("com.cedricziel.folio") == true
            ? FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: SharedConfiguration.appGroup)
            : nil
    }

    private let fileURL: URL?
    private let lockURL: URL?
    private let now: @Sendable () -> Date
    private let lock = NSLock()
    private var memory = State()

    /// Without a directory the session lives in this process only.
    init(directory: URL?, now: @escaping @Sendable () -> Date = { Date() }) {
        fileURL = directory?.appending(path: "telemetry-session.json")
        lockURL = directory?.appending(path: "telemetry-session.lock")
        self.now = now
    }

    /// The live session ID, without counting as activity.
    func current() -> String? {
        withState(write: false) { state, now in
            guard state.signedIn, let session = state.session, Self.expiry(session, at: now) == nil else {
                return nil
            }
            return session.id
        }
    }

    /// Whether the app holds a signed-in account, as far as the shared record knows.
    func isSignedIn() -> Bool {
        withState(write: false) { state, _ in state.signedIn }
    }

    /// Marks the shared record signed in. Called by the app once it holds a signed-in session.
    func signIn() {
        withState(write: true) { state, _ in state.signedIn = true }
    }

    /// Records user activity: extends the live session, or ends an expired one and starts the next.
    /// While signed out there is no session to extend or start.
    func touch() -> (id: String?, boundaries: [SessionBoundary]) {
        withState(write: true) { state, now in
            guard state.signedIn else { return (id: nil, boundaries: []) }
            var record = state.session
            defer { state.session = record }
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

    /// Ends the session and forgets it, so the next one is not linked to it. Sign-out also marks the record
    /// signed out, so no process can start another session until the app signs in again.
    func end(_ reason: SessionEndReason) -> [SessionBoundary] {
        withState(write: true) { state, now in
            let live = state.session.flatMap { Self.expiry($0, at: now) == nil ? $0 : nil }
            state.session = nil
            if reason == .signOut { state.signedIn = false }
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

    private func withState<T>(write: Bool, _ body: (inout State, Date) -> T) -> T {
        lock.withLock {
            withFileLock {
                var state = read()
                let original = state
                let result = body(&state, now())
                if write, state != original { save(state) }
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

    /// A missing or unreadable record reads as signed out, until the app signs in.
    private func read() -> State {
        guard let fileURL else { return memory }
        guard let data = try? Data(contentsOf: fileURL) else { return State() }
        return (try? JSONDecoder().decode(State.self, from: data)) ?? State()
    }

    private func save(_ state: State) {
        guard let fileURL else {
            memory = state
            return
        }
        if let data = try? JSONEncoder().encode(state) {
            try? data.write(to: fileURL, options: .atomic)
        }
    }
}
