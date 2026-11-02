import Foundation

/// Memory-only thumbnails for one signed-in workspace and vault. Failed loads are never cached.
public actor ThumbnailCache {
    private struct Key: Hashable {
        let vaultID: UUID
        let documentID: UUID
        let revision: Date
    }

    private struct Pending {
        let task: Task<Data?, any Error>
        var waiters: Set<UUID>
    }

    private let service: any FolioService
    private let vaultID: UUID
    private let byteLimit: Int
    private var cached: [Key: Data] = [:]
    private var recent: [Key] = []
    private var byteCount = 0
    private var pending: [Key: Pending] = [:]

    public init(service: any FolioService, vaultID: UUID, byteLimit: Int = 8 * 1_024 * 1_024) {
        self.service = service
        self.vaultID = vaultID
        self.byteLimit = max(0, byteLimit)
    }

    /// Coalesces concurrent loads and cancels the transfer when its last caller leaves.
    public func data(for document: Document) async throws -> Data? {
        try Task.checkCancellation()
        let key = Key(vaultID: vaultID, documentID: document.id, revision: document.updatedAt)
        if let data = cached[key] {
            touch(key)
            return data
        }
        let waiter = UUID()
        let task: Task<Data?, any Error>
        if var existing = pending[key] {
            existing.waiters.insert(waiter)
            pending[key] = existing
            task = existing.task
        } else {
            let service = service
            task = Task { try await service.thumbnail(id: document.id) }
            pending[key] = Pending(task: task, waiters: [waiter])
        }
        do {
            let data = try await withTaskCancellationHandler {
                let data = try await task.value
                try Task.checkCancellation()
                return data
            } onCancel: {
                Task { await self.release(waiter, for: key) }
            }
            release(waiter, for: key)
            if let data, data.count <= byteLimit {
                if let old = cached.updateValue(data, forKey: key) { byteCount -= old.count }
                byteCount += data.count
                touch(key)
                while byteCount > byteLimit, let oldest = recent.first {
                    recent.removeFirst()
                    byteCount -= cached.removeValue(forKey: oldest)?.count ?? 0
                }
            }
            return data
        } catch {
            release(waiter, for: key)
            throw error
        }
    }

    private func touch(_ key: Key) {
        recent.removeAll { $0 == key }
        recent.append(key)
    }

    private func release(_ waiter: UUID, for key: Key) {
        guard var existing = pending[key], existing.waiters.remove(waiter) != nil else { return }
        if existing.waiters.isEmpty {
            pending.removeValue(forKey: key)
            existing.task.cancel()
        } else {
            pending[key] = existing
        }
    }
}
