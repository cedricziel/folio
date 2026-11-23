import FolioKit
import SwiftUI

private actor MetadataFixtureService: CustomMetadataService {
    let documentID = UUID()
    let linkedID = UUID()
    let amountID = UUID()
    let linkID = UUID()
    var amount = CustomFieldValue.decimal("9007199254740993.01")
    var revision = UUID()

    func customFields(after: UUID?) async throws -> [CustomFieldDefinition] {
        guard after == nil else { return [] }
        return [
            .init(id: amountID, draft: .init(name: "Exact amount", kind: .decimal), isRetired: false),
            .init(id: linkID, draft: .init(name: "Receipt", kind: .documentLink), isRetired: false),
        ]
    }

    func customMetadata(documentID: UUID) async throws -> CustomMetadataSnapshot {
        guard documentID == self.documentID else { throw FolioError.unreadableResponse }
        return CustomMetadataSnapshot(
            revision: revision,
            values: [
                .init(fieldID: amountID, value: amount),
                .init(
                    fieldID: linkID, value: .documentLink(linkedID),
                    link: .init(title: "Linked receipt", isTrashed: false)),
            ]
        )
    }

    func setCustomValue(
        documentID: UUID, revision: UUID?, fieldID: UUID, value: CustomFieldValue?
    ) async throws -> CustomMetadataSnapshot {
        guard documentID == self.documentID, revision == self.revision, fieldID == amountID,
            case .decimal? = value, let value
        else { throw FolioError.server(status: 409, message: "Value changed") }
        amount = value
        self.revision = UUID()
        return try await customMetadata(documentID: documentID)
    }

    func saveCustomField(id: UUID?, draft: CustomFieldDraft) async throws -> CustomFieldDefinition {
        throw FolioError.unreadableResponse
    }

    func setCustomFieldRetired(id: UUID, retired: Bool) async throws { throw FolioError.unreadableResponse }

    func customOption(fieldID: UUID, id: UUID) async throws -> CustomFieldOption {
        throw FolioError.unreadableResponse
    }

    func customOptions(fieldID: UUID, after: UUID?) async throws -> [CustomFieldOption] { [] }

    func saveCustomOption(fieldID: UUID, id: UUID?, label: String, position: UInt32) async throws -> CustomFieldOption {
        throw FolioError.unreadableResponse
    }

    func setCustomOptionRetired(fieldID: UUID, id: UUID, retired: Bool) async throws {
        throw FolioError.unreadableResponse
    }
}

struct CustomMetadataFixture: View {
    let documents: any FolioService
    let canEdit: Bool
    @State private var metadata = MetadataFixtureService()
    @State private var showing = false
    @State private var opened = false

    var body: some View {
        VStack {
            Button("Open custom fields") { showing = true }
            if opened { Text("Opened linked receipt") }
        }
        .sheet(isPresented: $showing) {
            CustomMetadataView(
                documentID: metadata.documentID, metadata: metadata, documents: documents, canEdit: canEdit
            ) { id in
                opened = id == metadata.linkedID
            }
        }
    }
}
