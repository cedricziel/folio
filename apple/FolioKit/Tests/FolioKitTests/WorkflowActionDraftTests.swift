import FolioAPI
import Foundation
import Testing

@testable import FolioKit

struct WorkflowActionDraftTests {
    @Test func movesSavedRowsAndAssignsNoSourceToNewRows() throws {
        var draft = WorkflowActionDraft(
            revision: 7,
            actions: [
                .case1(
                    .init(
                        headers: .init(additionalProperties: ["Authorization": "********"]),
                        secret: "********", _type: .webhook, url: "https://first.test/hook")),
                .case1(.init(secret: "********", _type: .webhook, url: "https://second.test/hook")),
            ]
        )
        let second = try #require(draft.rows.last?.id)
        draft.move(second, by: -1)
        var new = WorkflowActionDraft.Row(kind: .addTags)
        new.references = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        draft.rows.append(new)

        let prepared = try draft.prepare(for: .document)
        #expect(draft.rows.first?.id == second)
        #expect(prepared.sources == ["1", "0", "-"])
        #expect(
            prepared.actions == [
                .case1(.init(method: .post, secret: "********", _type: .webhook, url: "https://second.test/hook")),
                .case1(
                    .init(
                        headers: .init(additionalProperties: ["Authorization": "********"]),
                        method: .post, secret: "********", _type: .webhook, url: "https://first.test/hook")),
                .case2(.init(tagIds: ["aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"], _type: .addTags)),
            ])
    }

    @Test func movedWebhookMasksCannotFollowAChangedDestination() throws {
        var draft = WorkflowActionDraft(
            revision: 5,
            actions: [
                .case1(.init(secret: "stored credential", _type: .webhook, url: "https://first.test/hook"))
            ]
        )
        #expect(draft.rows[0].signingSecret == "********")
        draft.rows[0].url = "https://second.test/hook"
        #expect(throws: FolioError.self) { _ = try draft.prepare(for: .document) }
        draft.rows[0].signingSecret = "replacement"
        let prepared = try draft.prepare(for: .document)
        #expect(prepared.sources == ["0"])
        #expect(
            prepared.actions == [
                .case1(.init(method: .post, secret: "replacement", _type: .webhook, url: "https://second.test/hook"))
            ])
    }

    @Test func roundTripsExpandedDocumentActionsWithTheirSourcePositions() throws {
        let actions: [WorkflowActionDraft.Action] = [
            .case6(.init(title: "Invoice", _type: .renameDocument)),
            .case7(.init(tagIds: ["aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"], _type: .removeTags)),
            .case8(.init(_type: .clearCorrespondent)),
            .case9(.init(_type: .clearDocumentType)),
            .case11(.init(date: "2028-02-29", _type: .setDocumentDate)),
            .case12(.init(_type: .clearDocumentDate)),
            .case13(.init(reviewed: false, _type: .setReviewed)),
            .case10(.init(_type: .trashDocument)),
        ]
        let draft = WorkflowActionDraft(revision: 9, actions: actions)

        let prepared = try draft.prepare(for: .document)

        #expect(prepared.actions == actions)
        #expect(prepared.sources == ["0", "1", "2", "3", "4", "5", "6", "7"])
    }

    @Test func moveToTrashMustBeTheFinalAction() {
        let draft = WorkflowActionDraft(actions: [
            .case10(.init(_type: .trashDocument)),
            .case13(.init(reviewed: true, _type: .setReviewed)),
        ])

        #expect(draft.validationMessage(for: .document) == "Move to Trash must be the final action.")
    }

    @Test func orderedCustomFieldActionsPreserveEveryTypedValue() throws {
        let field = try #require(UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"))
        let reference = try #require(UUID(uuidString: "dddddddd-eeee-4fff-8aaa-bbbbbbbbbbbb"))
        let fieldID = field.uuidString.lowercased()
        let values: [CustomFieldValue] = [
            .text(""), .longText("First line\nSecond line"), .boolean(false), .integer("9223372036854775807"),
            .decimal("9007199254740993.010"), .date("2028-02-29"), .money(amount: "123.4500", currency: "EUR"),
            .select(reference), .url("https://example.test/receipt"), .documentLink(reference),
        ]
        let original = values.map { value in
            WorkflowActionDraft.Action.case14(.init(fieldId: fieldID, _type: .setMetadata, value: value.apiValue))
        }
        var draft = WorkflowActionDraft(revision: 13, actions: original)
        let first = try #require(draft.rows.first?.id)
        draft.move(first, by: 1)
        var clear = WorkflowActionDraft.Row(kind: .clearMetadata)
        #expect(throws: FolioError.self) { _ = try clear.prepare() }
        clear.metadataFieldID = field
        draft.rows.append(clear)
        let prepared = try draft.prepare(for: .document)
        #expect(prepared.sources == ["1", "0", "2", "3", "4", "5", "6", "7", "8", "9", "-"])
        #expect(
            prepared.actions == [original[1], original[0]] + Array(original.dropFirst(2)) + [
                .case15(.init(fieldId: fieldID, _type: .clearMetadata))
            ])
        #expect(throws: FolioError.self) { _ = try draft.prepare(for: .event) }
    }

    @Test func incompatibleAndEmptyActionsCannotBeSaved() throws {
        let draft = WorkflowActionDraft(revision: 2, actions: [.case5(.init(_type: .skipImport))])
        #expect(try draft.prepare(for: .ingestCandidate).sources == ["0"])
        #expect(throws: FolioError.self) { _ = try draft.prepare(for: .document) }
        let empty = WorkflowActionDraft(revision: 2, actions: [])
        #expect(throws: FolioError.self) { _ = try empty.prepare(for: .event) }

        var duplicateHeaders = WorkflowActionDraft(revision: 2, actions: [])
        var row = WorkflowActionDraft.Row(kind: .webhook)
        row.url = "https://first.test/hook"
        row.headerLines = "Authorization: first\nAuthorization: second"
        duplicateHeaders.rows.append(row)
        #expect(throws: FolioError.self) { _ = try duplicateHeaders.prepare(for: .event) }
    }
}
