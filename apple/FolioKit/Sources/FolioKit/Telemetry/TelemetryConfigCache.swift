import Foundation

/// The server whose relay settings were last accepted, kept in the App Group so a new extension process records
/// from its first request instead of waiting for `/api/telemetry/config`. Only the server and when it said yes are
/// kept: exports still go through the process's own credentials, and each process asks the server again.
struct TelemetryConfigCache: Sendable {
    static let lifetime: TimeInterval = 24 * 60 * 60

    static let shared = TelemetryConfigCache(directory: TelemetrySession.sharedDirectory)

    private struct Entry: Codable {
        var server: URL
        var acceptedAt: Date
    }

    private let fileURL: URL?
    private let now: @Sendable () -> Date

    /// Without a directory nothing is cached.
    init(directory: URL?, now: @escaping @Sendable () -> Date = { Date() }) {
        fileURL = directory?.appending(path: "telemetry-config.json")
        self.now = now
    }

    /// Whether `server` enabled the relay within `lifetime`. A clock set back counts as expired.
    func enabled(for server: URL) -> Bool {
        guard let entry = read(), entry.server == server else { return false }
        let age = now().timeIntervalSince(entry.acceptedAt)
        return age >= 0 && age < Self.lifetime
    }

    func accept(_ server: URL) {
        guard let fileURL, let data = try? JSONEncoder().encode(Entry(server: server, acceptedAt: now())) else {
            return
        }
        try? data.write(to: fileURL, options: .atomic)
    }

    /// Forgets `server`'s settings, or any server's when nil.
    func clear(_ server: URL? = nil) {
        guard let fileURL, server == nil || read()?.server == server else { return }
        try? FileManager.default.removeItem(at: fileURL)
    }

    private func read() -> Entry? {
        guard let fileURL, let data = try? Data(contentsOf: fileURL) else { return nil }
        return try? JSONDecoder().decode(Entry.self, from: data)
    }
}
