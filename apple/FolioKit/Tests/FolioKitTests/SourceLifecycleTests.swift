import Foundation
import Testing

@testable import FolioKit

@MainActor struct SourceLifecycleTests {
    @Test func onlyOwnersCanChangeManagedSourcesWithLifecycleSupport() async {
        let service = SourceLifecycleFake()
        let source = await service.source
        let reader = SourceLifecycleViewModel(source: source, service: service, canManage: false)
        await reader.setEnabled(false)
        #expect(!reader.canChange)
        #expect(reader.source.enabled)
        #expect(await service.source.enabled)
        let configured = SourceActivity(
            id: source.id, vaultID: source.vaultID, name: source.name, health: .available,
            lastCheckedAt: nil, origin: .config, enabled: true)
        let owner = SourceLifecycleViewModel(source: configured, service: service, canManage: true)
        await owner.trash()
        #expect(!owner.canChange)
        #expect(owner.source.deletedAt == nil)
    }

    @Test func legacyMetadataDoesNotOfferPotentiallyPermanentDeletion() async {
        let service = SourceLifecycleFake()
        let source = await service.source
        let legacy = SourceActivity(
            id: source.id, vaultID: source.vaultID, name: source.name, health: .available,
            lastCheckedAt: nil, origin: .managed, enabled: true)
        let model = SourceLifecycleViewModel(source: legacy, service: service, canManage: true)
        await model.trash()
        #expect(!model.canChange)
        #expect(!model.didTrash)
        #expect(await service.source.deletedAt == nil)
    }

    @Test func disableTrashAndRestoreKeepSourceIdentityAndAuditMetadata() async {
        let service = SourceLifecycleFake()
        let model = SourceLifecycleViewModel(source: await service.source, service: service, canManage: true)
        await model.setEnabled(false)
        #expect(!model.source.enabled)
        #expect(model.source.health == .off)
        await model.trash()
        #expect(model.didTrash)
        #expect(!model.canChange)
        let retained = await service.source
        #expect(retained.deletedAt != nil)
        let trash = SourceLifecycleViewModel(source: retained, service: service, canManage: true)
        #expect(trash.canRestore)
        await trash.restore()
        #expect(trash.source.deletedAt == nil)
        #expect(!trash.source.enabled)
        #expect(trash.source.createdBy == "fixture-owner")
    }

    @Test func restoreConflictKeepsTrashedSourceAndShowsSafeRecovery() async {
        let service = SourceLifecycleFake()
        await service.setConflict()
        let model = SourceLifecycleViewModel(source: await service.source, service: service, canManage: true)
        await model.restore()
        #expect(model.source.deletedAt != nil)
        #expect(model.canRestore)
        #expect(
            model.error
                == "Another active source uses this name or occupies the Paperless slot. Resolve the conflict in web settings, then try again."
        )
        #expect(!model.isSaving)
    }
}

private actor SourceLifecycleFake: SourceLifecycleService {
    private let id = UUID()
    private let vault = UUID()
    private var enabled = true
    private var deletedAt: Date?
    private var conflicts = false
    var source: SourceActivity {
        SourceActivity(
            id: id, vaultID: vault, name: "Mailbox", health: enabled ? .available : .off,
            lastCheckedAt: nil, origin: .managed, enabled: enabled,
            createdAt: Date(timeIntervalSince1970: 1), updatedAt: Date(timeIntervalSince1970: 2),
            deletedAt: deletedAt, createdBy: "fixture-owner", updatedBy: "fixture-owner")
    }
    func setConflict() { conflicts = true; deletedAt = Date(timeIntervalSince1970: 3) }
    func sourceActivity(vaultID: UUID) async throws -> [SourceActivity] { [source] }
    func sourceActivity(vaultID: UUID, deleted: Bool) async throws -> [SourceActivity] { [source] }
    func setSourceEnabled(vaultID: UUID, sourceID: UUID, enabled: Bool) async throws -> SourceActivity {
        self.enabled = enabled
        return source
    }
    func trashSource(vaultID: UUID, sourceID: UUID) async throws { deletedAt = Date(timeIntervalSince1970: 3) }
    func restoreSource(vaultID: UUID, sourceID: UUID) async throws -> SourceActivity {
        if conflicts { throw FolioError.server(status: 409, message: "password=private") }
        deletedAt = nil
        return source
    }
    func sourceOutcomes(vaultID: UUID, sourceID: UUID, before: Int64?) async throws -> SourceOutcomePage {
        SourceOutcomePage(vaultID: vaultID, sourceID: sourceID, outcomes: [])
    }
}
