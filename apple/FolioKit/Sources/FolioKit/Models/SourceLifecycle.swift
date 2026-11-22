import Foundation

public protocol SourceLifecycleService: SourceActivityService {
    func sourceActivity(vaultID: UUID, deleted: Bool) async throws -> [SourceActivity]
    func setSourceEnabled(vaultID: UUID, sourceID: UUID, enabled: Bool) async throws -> SourceActivity
    func trashSource(vaultID: UUID, sourceID: UUID) async throws
    func restoreSource(vaultID: UUID, sourceID: UUID) async throws -> SourceActivity
}
