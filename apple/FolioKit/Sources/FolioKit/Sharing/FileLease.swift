import Foundation

/// A lock the app and its extensions share through files in the App Group container.
///
/// Each holder owns one generation file, `<name>.<generation>`, and the highest generation holds the lease while
/// it is renewed within `timeout`. Taking over a released or abandoned lease means creating the next generation,
/// which only one process can do, and nobody ever moves or deletes the file of the generation in charge: releasing
/// only ages the holder's own file. So a slow holder, a late release and two processes taking over at once never
/// leave two holders.
struct FileLease: Sendable {
    let file: URL
    let timeout: TimeInterval

    /// Takes the lease when it is free, released or not renewed within `timeout`. Returns the generation that holds
    /// it, or nil while another holder keeps it.
    func acquire(now: Date = .now) -> Int? {
        let generations = existing()
        let current = generations.max()
        if let current, let modified = Self.modified(path(current)), modified >= now.addingTimeInterval(-timeout) {
            return nil
        }
        let next = (current ?? 0) + 1
        do {
            try FileManager.default.createDirectory(
                at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data().write(to: path(next), options: .withoutOverwriting)
        } catch {
            return nil
        }
        // A process that listed before an older generation was cleaned up can recreate it; only the highest holds.
        guard existing().allSatisfy({ $0 <= next }) else {
            try? FileManager.default.removeItem(at: path(next))
            return nil
        }
        for older in generations where older < next {
            try? FileManager.default.removeItem(at: path(older))
        }
        return next
    }

    /// Marks the lease held by `generation` as alive. False when a later generation has taken it over.
    @discardableResult
    func renew(_ generation: Int, now: Date = .now) -> Bool {
        guard existing().allSatisfy({ $0 <= generation }) else { return false }
        return (try? FileManager.default.setAttributes([.modificationDate: now], ofItemAtPath: path(generation).path))
            != nil
    }

    /// Frees the lease by aging the holder's own file, so the next caller takes over with a new generation.
    func release(_ generation: Int) {
        try? FileManager.default.setAttributes(
            [.modificationDate: Date(timeIntervalSince1970: 0)], ofItemAtPath: path(generation).path)
    }

    /// Deletes every generation, for a record that is gone, and the single lock file earlier builds used.
    func removeAll() {
        try? FileManager.default.removeItem(at: file)
        for generation in existing() {
            try? FileManager.default.removeItem(at: path(generation))
        }
    }

    private func path(_ generation: Int) -> URL {
        file.deletingLastPathComponent().appending(path: "\(file.lastPathComponent).\(generation)")
    }

    private func existing() -> [Int] {
        let prefix = file.lastPathComponent + "."
        let names = (try? FileManager.default.contentsOfDirectory(atPath: file.deletingLastPathComponent().path)) ?? []
        return names.compactMap { name in
            name.hasPrefix(prefix) ? Int(name.dropFirst(prefix.count)) : nil
        }
    }

    private static func modified(_ file: URL) -> Date? {
        (try? FileManager.default.attributesOfItem(atPath: file.path))?[.modificationDate] as? Date
    }
}
