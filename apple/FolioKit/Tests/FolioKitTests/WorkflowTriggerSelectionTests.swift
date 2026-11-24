import FolioAPI
import Foundation
import Testing

@testable import FolioKit

struct WorkflowTriggerSelectionTests {
    @Test func generatedInputAcceptsAnOrderedListWithoutTheLegacyTrigger() throws {
        let ordered = ["document.title_changed", "document.tags_changed"]
        let json = """
            {"name":"Titles or tags","triggers":["document.title_changed","document.tags_changed"],
             "actions":[{"type":"webhook","url":"https://receiver.test"}]}
            """
        let input = try JSONDecoder().decode(Components.Schemas.WorkflowInput.self, from: Data(json.utf8))
        #expect(input.trigger == nil)
        #expect(input.triggers?.map(\.rawValue) == ordered)
        let data = try JSONEncoder().encode(input)
        let sent = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(sent["triggers"] as? [String] == ordered)
        #expect(sent["trigger"] == nil)
    }

    @Test func editorUsesTheActionAndConditionIntersection() throws {
        let actor = WorkflowTriggerChoice.ConditionField(id: "event.actor", title: "Actor", valueType: "identifier")
        let filename = WorkflowTriggerChoice.ConditionField(id: "filename_glob", title: "File name", valueType: "text")
        let document = WorkflowTriggerChoice(
            id: "document.uploaded", title: "Upload", target: .document, conditionFields: [actor, filename])
        let deleted = WorkflowTriggerChoice(
            id: "document.deleted", title: "Delete", target: .event, conditionFields: [actor])

        let choice = try #require(WorkflowTriggerChoice.matching([document.id, deleted.id], in: [document, deleted]))

        #expect(choice.id == document.id)
        #expect(choice.target == .event)
        #expect(choice.conditionFields == [actor])
        #expect(WorkflowConditionTree.Field.available(for: choice, includeLegacyDefaults: false).isEmpty)
    }

    @Test func newAndExistingBuilderRejectDuplicateAndIncompatibleSelections() {
        let document = WorkflowTriggerChoice(
            id: "document.uploaded", title: "Upload", target: .document, conditionFields: [])
        let singleOnlyChoices = [
            WorkflowTriggerChoice(
                id: "ingest.received", title: "Before import", target: .ingestCandidate, conditionFields: []),
            WorkflowTriggerChoice(id: "workflow.scheduled", title: "Clock", target: .event, conditionFields: []),
            WorkflowTriggerChoice(
                id: "document.date_offset", title: "Date offset", target: .document, conditionFields: []),
        ]
        let catalog = [document] + singleOnlyChoices
        #expect(WorkflowTriggerChoice.matching([document.id, document.id], in: catalog) == nil)
        #expect(WorkflowTriggerChoice.matching([document.id, ""], in: catalog) == nil)
        for choice in singleOnlyChoices {
            #expect(WorkflowTriggerChoice.matching([choice.id], in: catalog) == choice)
            #expect(WorkflowTriggerChoice.matching([choice.id, document.id], in: catalog) == nil)
            #expect(WorkflowTriggerChoice.matching([document.id, choice.id], in: catalog) == nil)
        }
    }

    @Test func generatedInputStillAcceptsTheLegacyTriggerWithoutAList() throws {
        let json = """
            {"name":"Titles","trigger":"document.title_changed",
             "actions":[{"type":"webhook","url":"https://receiver.test"}]}
            """
        let input = try JSONDecoder().decode(Components.Schemas.WorkflowInput.self, from: Data(json.utf8))
        #expect(input.trigger?.rawValue == "document.title_changed")
        #expect(input.triggers == nil)
    }
}
