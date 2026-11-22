import FolioKit
import SwiftUI

struct SourceLifecycleFixture: View {
    private let service: SourceLifecycleFixtureService
    private let readOnly: Bool
    private let showsConflictControl: Bool
    @State private var conflictResolved = false

    init() {
        let arguments = ProcessInfo.processInfo.arguments
        readOnly = arguments.contains("--read-only")
        showsConflictControl = arguments.contains("--source-conflict")
        service = SourceLifecycleFixtureService(
            failInitialLoad: arguments.contains("--initial-load-error"), conflicts: showsConflictControl)
    }

    var body: some View {
        VStack(spacing: FolioTheme.Spacing.space2) {
            SourceActivityView(vaultID: SourceLifecycleFixtureService.vaultID, service: service, canManage: !readOnly) {
                _ in
            }
            if showsConflictControl {
                Button(conflictResolved ? "Fixture conflict resolved" : "Resolve fixture source conflict") {
                    Task {
                        await service.resolveConflict(); conflictResolved = true
                    }
                }
                .padding(FolioTheme.Spacing.space2)
            }
        }
        .dynamicTypeSize(ProcessInfo.processInfo.arguments.contains("--large-text") ? .accessibility3 : .large)
    }
}

private actor SourceLifecycleFixtureService: SourceLifecycleService {
    static let vaultID = UUID(uuidString: "11111111-1111-4111-8111-111111111111")!
    private let mailboxID = UUID(uuidString: "22222222-2222-4222-8222-222222222222")!
    private let retainedID = UUID(uuidString: "33333333-3333-4333-8333-333333333333")!
    private let configuredID = UUID(uuidString: "44444444-4444-4444-8444-444444444444")!
    private var failInitialLoad: Bool
    private var conflicts: Bool
    private var mailboxEnabled = true
    private var mailboxDeleted = false
    private var retainedDeleted = true

    init(failInitialLoad: Bool, conflicts: Bool) {
        self.failInitialLoad = failInitialLoad
        self.conflicts = conflicts
    }

    func resolveConflict() { conflicts = false }

    func sourceActivity(vaultID: UUID) async throws -> [SourceActivity] {
        try await sourceActivity(vaultID: vaultID, deleted: false)
    }

    func sourceActivity(vaultID: UUID, deleted: Bool) async throws -> [SourceActivity] {
        guard vaultID == Self.vaultID else { throw FolioError.unreadableResponse }
        if failInitialLoad {
            failInitialLoad = false
            throw FolioError.unreachable
        }
        return sources.filter { ($0.deletedAt != nil) == deleted }
    }

    func setSourceEnabled(vaultID: UUID, sourceID: UUID, enabled: Bool) async throws -> SourceActivity {
        guard vaultID == Self.vaultID, sourceID == mailboxID else { throw FolioError.unreadableResponse }
        mailboxEnabled = enabled
        return sources.first { $0.id == sourceID }!
    }

    func trashSource(vaultID: UUID, sourceID: UUID) async throws {
        guard vaultID == Self.vaultID, sourceID == mailboxID else { throw FolioError.unreadableResponse }
        mailboxDeleted = true
    }

    func restoreSource(vaultID: UUID, sourceID: UUID) async throws -> SourceActivity {
        guard vaultID == Self.vaultID else { throw FolioError.unreadableResponse }
        if conflicts { throw FolioError.server(status: 409, message: "private provider message") }
        if sourceID == mailboxID {
            mailboxDeleted = false
        } else if sourceID == retainedID {
            retainedDeleted = false
        } else {
            throw FolioError.unreadableResponse
        }
        return sources.first { $0.id == sourceID }!
    }

    func sourceOutcomes(vaultID: UUID, sourceID: UUID, before: Int64?) async throws -> SourceOutcomePage {
        SourceOutcomePage(
            vaultID: vaultID, sourceID: sourceID,
            outcomes: [
                SourceOutcome(
                    id: 1, itemID: mailboxID, attemptID: retainedID,
                    occurredAt: Date(timeIntervalSince1970: 1_790_000_000),
                    outcome: .imported)
            ])
    }

    private var sources: [SourceActivity] {
        let created = Date(timeIntervalSince1970: 1_780_000_000)
        let updated = Date(timeIntervalSince1970: 1_790_000_000)
        return [
            SourceActivity(
                id: mailboxID, vaultID: Self.vaultID, name: "Mailbox", health: mailboxEnabled ? .available : .off,
                lastCheckedAt: updated, origin: .managed, enabled: mailboxEnabled,
                createdAt: created, updatedAt: updated, deletedAt: mailboxDeleted ? updated : nil,
                createdBy: "Taylor", updatedBy: "Morgan"),
            SourceActivity(
                id: retainedID, vaultID: Self.vaultID, name: "Archived mailbox", health: .off,
                lastCheckedAt: updated, origin: .managed, enabled: false,
                createdAt: created, updatedAt: updated, deletedAt: retainedDeleted ? updated : nil,
                createdBy: nil, updatedBy: nil),
            SourceActivity(
                id: configuredID, vaultID: Self.vaultID, name: "Server folder", health: .notChecked,
                lastCheckedAt: nil, origin: .config, enabled: true),
        ]
    }
}
