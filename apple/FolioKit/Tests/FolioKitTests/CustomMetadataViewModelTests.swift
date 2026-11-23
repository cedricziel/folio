import Foundation
import Testing

@testable import FolioKit

@MainActor
struct CustomMetadataViewModelTests {
    @Test func aReloadSerializesSnapshotReadsAndWrites() async throws {
        let service = MetadataReloadFixture()
        let field = UUID()
        let model = CustomMetadataViewModel(documentID: UUID(), service: service)
        await model.load()
        await service.pauseNextRead()

        let reload = Task { try await model.reload() }
        await service.waitForPausedRead()
        await #expect(throws: FolioError.self) {
            try await model.save(fieldID: field, value: .decimal("9007199254740993.01"))
        }
        try await model.reload()
        #expect(await service.reads == 2)
        #expect(await service.writes == 0)
        await service.resumeRead()
        try await reload.value

        try await model.save(fieldID: field, value: .decimal("9007199254740993.01"))
        #expect(model.snapshot.values == [.init(fieldID: field, value: .decimal("9007199254740993.01"))])
    }
}

private actor MetadataReloadFixture: CustomMetadataService {
    private(set) var reads = 0
    private(set) var writes = 0
    private var snapshot = CustomMetadataSnapshot(revision: UUID(), values: [])
    private var pause = false
    private var waitingForRead: CheckedContinuation<Void, Never>?
    private var activeRead: CheckedContinuation<Void, Never>?

    func pauseNextRead() { pause = true }

    func waitForPausedRead() async {
        if activeRead != nil { return }
        await withCheckedContinuation { waitingForRead = $0 }
    }

    func resumeRead() {
        activeRead?.resume()
        activeRead = nil
    }

    func customMetadata(documentID: UUID) async throws -> CustomMetadataSnapshot {
        reads += 1
        let captured = snapshot
        if pause {
            pause = false
            await withCheckedContinuation {
                activeRead = $0
                waitingForRead?.resume()
                waitingForRead = nil
            }
        }
        return captured
    }

    func setCustomValue(
        documentID: UUID, revision: UUID?, fieldID: UUID, value: CustomFieldValue?
    ) async throws -> CustomMetadataSnapshot {
        writes += 1
        snapshot = .init(revision: UUID(), values: [.init(fieldID: fieldID, value: value)])
        return snapshot
    }

    func customFields(after: UUID?) async throws -> [CustomFieldDefinition] { [] }
    func customOptions(fieldID: UUID, after: UUID?) async throws -> [CustomFieldOption] { [] }

    func saveCustomField(id: UUID?, draft: CustomFieldDraft) async throws -> CustomFieldDefinition {
        throw FolioError.unreadableResponse
    }

    func setCustomFieldRetired(id: UUID, retired: Bool) async throws { throw FolioError.unreadableResponse }

    func customOption(fieldID: UUID, id: UUID) async throws -> CustomFieldOption {
        throw FolioError.unreadableResponse
    }

    func saveCustomOption(fieldID: UUID, id: UUID?, label: String, position: UInt32) async throws -> CustomFieldOption {
        throw FolioError.unreadableResponse
    }

    func setCustomOptionRetired(fieldID: UUID, id: UUID, retired: Bool) async throws {
        throw FolioError.unreadableResponse
    }
}
