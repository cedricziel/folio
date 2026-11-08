import Foundation

/// A lock the app and its extensions share through one file in the App Group container. The file holds the
/// holder's token, and a holder that stops renewing it for `timeout` (its process died) can be taken over.
struct FileLease: Sendable {
    let file: URL
    let timeout: TimeInterval

    /// Creates the lease file, or takes over one not renewed within `timeout`. Returns nil while another holder
    /// keeps it.
    func acquire(now: Date = .now) -> UUID? {
        let token = UUID()
        for _ in 0..<2 {
            if create(token) {
                return token
            }
            guard breakIfStale(now: now) else { return nil }
        }
        return nil
    }

    /// Marks the lease held by `token` as alive. False when it was taken over or released.
    @discardableResult
    func renew(_ token: UUID, now: Date = .now) -> Bool {
        guard holder() == token else { return false }
        return (try? FileManager.default.setAttributes([.modificationDate: now], ofItemAtPath: file.path)) != nil
    }

    /// Removes the lease, unless another holder has taken it over since.
    func release(_ token: UUID) {
        guard holder() == token else { return }
        try? FileManager.default.removeItem(at: file)
    }

    private func create(_ token: UUID) -> Bool {
        do {
            try FileManager.default.createDirectory(
                at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data(token.uuidString.utf8).write(to: file, options: .withoutOverwriting)
            return true
        } catch {
            return false
        }
    }

    private func holder() -> UUID? {
        (try? String(contentsOf: file, encoding: .utf8)).flatMap(UUID.init(uuidString:))
    }

    /// Moves a stale lease file aside. A rename succeeds for only one process, so two processes that both found
    /// the lease stale never both remove it and then both create it. (A third process creating the file while a
    /// fresh one is being put back could still slip in; the app and one share sheet never get there.) True when the
    /// caller may try to create the lease again.
    private func breakIfStale(now: Date) -> Bool {
        let cutoff = now.addingTimeInterval(-timeout)
        guard let modified = Self.modified(file) else { return true }
        guard modified < cutoff else { return false }
        let aside = file.deletingLastPathComponent()
            .appending(path: "\(file.lastPathComponent).\(UUID().uuidString).stale")
        do {
            try FileManager.default.moveItem(at: file, to: aside)
        } catch {
            // Another process moved it first; whoever creates the file next holds the lease.
            return true
        }
        defer { try? FileManager.default.removeItem(at: aside) }
        if let moved = Self.modified(aside), moved >= cutoff {
            // Another process took the lease over between the check and the move: put its file back.
            try? FileManager.default.linkItem(at: aside, to: file)
            return false
        }
        return true
    }

    private static func modified(_ file: URL) -> Date? {
        (try? FileManager.default.attributesOfItem(atPath: file.path))?[.modificationDate] as? Date
    }
}
