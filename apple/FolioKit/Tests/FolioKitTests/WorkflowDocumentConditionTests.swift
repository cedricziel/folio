import Testing

@testable import FolioKit

struct WorkflowDocumentConditionTests {
    @Test func catalogRequiresTheAdvertisedFieldAndType() throws {
        let choice = WorkflowTriggerChoice(
            id: "document.date_changed", title: "Document date changed", target: .document,
            conditionFields: [
                .init(id: "event.document_change.before", title: "Previous date", valueType: "date"),
                .init(id: "event.document_change.after", title: "New date", valueType: "integer"),
                .init(id: "event.actor.user_id", title: "Actor", valueType: "text"),
            ]
        )
        var draft = try #require(WorkflowDocumentConditionDraft(choice))
        #expect(draft.fields.map(\.key) == [.before])
        draft[.before] = "2024-02-29"
        #expect(draft.validationMessage == nil)
        let object = try #require(draft.wireValue)
        #expect(object["type"] as? String == "date")
        #expect(object["before"] as? String == "2024-02-29")

        let olderCatalog = WorkflowTriggerChoice(
            id: choice.id, title: choice.title, target: choice.target, conditionFields: []
        )
        #expect(WorkflowDocumentConditionDraft(olderCatalog) == nil)
    }

    @Test func restoredRevisionUsesAnIntegerAndRejectsRevisionOne() throws {
        let choice = WorkflowTriggerChoice(
            id: "document.revision_restored", title: "File revision restored", target: .document,
            conditionFields: [
                .init(id: "event.document_change.number", title: "Revision number", valueType: "integer")
            ]
        )
        var draft = try #require(WorkflowDocumentConditionDraft(choice))
        draft[.number] = "1"
        #expect(draft.validationMessage != nil)
        draft[.number] = "3"
        #expect(draft.validationMessage == nil)
        #expect(draft.wireValue?["number"] as? Int == 3)
        draft[.number] = ""
        #expect(draft.wireValue == nil)
    }

    @Test func editingAnAdvertisedFieldKeepsStoredFieldsTheCatalogDoesNotExpose() throws {
        let choice = WorkflowTriggerChoice(
            id: "document.revision_created", title: "File revision created", target: .document,
            conditionFields: [
                .init(id: "event.document_change.source", title: "Revision source", valueType: "text")
            ]
        )
        var draft = try #require(WorkflowDocumentConditionDraft(choice))
        try draft.load(["type": "revision", "number": 3, "source": "replacement"])
        draft[.source] = ""

        let object = try #require(draft.wireValue)
        #expect(object["type"] as? String == "revision")
        #expect(object["number"] as? Int == 3)
        #expect(object["source"] == nil)
    }

    @Test func changingTriggerWithAnEmptyDraftKeepsTheExistingPredicate() throws {
        let choice = WorkflowTriggerChoice(
            id: "document.revision_created", title: "File revision created", target: .document,
            conditionFields: [
                .init(id: "event.document_change.number", title: "Revision number", valueType: "integer")
            ]
        )
        var draft = try #require(WorkflowDocumentConditionDraft(choice))

        #expect(!draft.shouldSubmit(startingTrigger: "document.title_changed"))
        #expect(draft.shouldSubmit(startingTrigger: choice.id))
        draft[.number] = "3"
        #expect(draft.shouldSubmit(startingTrigger: "document.title_changed"))
    }
}
