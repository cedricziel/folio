import Foundation
import Observation

@MainActor @Observable
public final class SourceLifecycleViewModel {
    public private(set) var source: SourceActivity
    public private(set) var isSaving = false
    public private(set) var didTrash = false
    public private(set) var error: String?
    private let service: any SourceLifecycleService
    private let canManage: Bool

    public init(source: SourceActivity, service: any SourceLifecycleService, canManage: Bool) {
        self.source = source
        self.service = service
        self.canManage = canManage
    }

    public var canChange: Bool {
        canManage && source.supportsLifecycle && source.deletedAt == nil && !didTrash
    }

    public var canRestore: Bool {
        canManage && source.supportsLifecycle && source.deletedAt != nil
    }

    public func setEnabled(_ enabled: Bool) async {
        guard canChange, !isSaving else { return }
        await save {
            try await service.setSourceEnabled(vaultID: source.vaultID, sourceID: source.id, enabled: enabled)
        }
    }

    public func trash() async {
        guard canChange, !isSaving else { return }
        isSaving = true
        error = nil
        defer { isSaving = false }
        do {
            try await service.trashSource(vaultID: source.vaultID, sourceID: source.id)
            didTrash = true
        } catch {
            self.error = message(for: error)
        }
    }

    public func restore() async {
        guard canRestore, !isSaving else { return }
        await save {
            try await service.restoreSource(vaultID: source.vaultID, sourceID: source.id)
        }
    }

    private func save(_ operation: () async throws -> SourceActivity) async {
        isSaving = true
        error = nil
        defer { isSaving = false }
        do {
            let updated = try await operation()
            guard updated.id == source.id, updated.vaultID == source.vaultID else {
                throw FolioError.unreadableResponse
            }
            source = updated
        } catch {
            self.error = message(for: error)
        }
    }

    private func message(for error: Error) -> String {
        switch error {
        case FolioError.server(status: 409, message: _):
            "Another active source uses this name or occupies the Paperless slot. Resolve the conflict in web settings, then try again."
        case FolioError.server(status: 403, message: _):
            "You don't have permission to change this source. Paperless sources also require an instance administrator."
        case FolioError.server(status: 404, message: _):
            "This source is no longer available. Refresh the source list."
        case FolioError.signInRequired:
            "Sign in again to change this source."
        default:
            "Could not change this source. Try again."
        }
    }
}
