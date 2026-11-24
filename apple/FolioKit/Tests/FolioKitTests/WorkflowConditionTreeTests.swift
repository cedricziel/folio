import Foundation
import OpenAPIRuntime
import Testing

@testable import FolioKit

struct WorkflowConditionTreeTests {
    @Test func editingNestedFieldsPreservesUnknownPredicatesAndEmptyAlternatives() throws {
        let original = """
            {
                "any": [],
                "all": [
                    {
                        "event": {"subject_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", "actor": {"type": "system"}},
                        "future_predicate": {"operand": null},
                        "none": [{"source": {"kind": "email", "folder_path_glob": "Receipts/*"}}]
                    }
                ]
            }
            """
        var tree = try decode(original)
        let subject = WorkflowConditionTree.Field(
            id: "event.subject_id", title: "Subject ID", isIdentifier: true
        )
        let folder = WorkflowConditionTree.Field(id: "source.folder_path_glob", title: "Folder path pattern")

        tree[.all]?[0][subject] = ""
        tree[.all]?[0][.none]?[0][folder] = "Invoices 2026/*"
        let expected = """
            {
                "any": [],
                "all": [
                    {
                        "event": {"actor": {"type": "system"}},
                        "future_predicate": {"operand": null},
                        "none": [{"source": {"kind": "email", "folder_path_glob": "Invoices 2026/*"}}]
                    }
                ]
            }
            """
        let desired = try decode(expected).payload()
        #expect(try tree.payload() == desired)

        tree[.any] = nil
        #expect(try tree.payload().value.keys.contains("any") == false)
    }

    @Test func validatesNestedIdentifiersAndGroupsAtServerLimits() throws {
        let idField = WorkflowConditionTree.Field(
            id: "tag_id", title: "Tag ID", isIdentifier: true
        )
        var tree = WorkflowConditionTree()
        tree[.all] = Array(repeating: WorkflowConditionTree(), count: 63)
        #expect(tree.count == 64)
        #expect(tree.validationMessage(fields: [idField]) == nil)
        tree[.all]?[0][idField] = "not an ID"
        #expect(tree.validationMessage(fields: [idField]) == "Enter a valid UUID for Tag ID.")
        tree[.all]?.append(.init())
        #expect(throws: FolioError.self) { _ = try tree.payload() }

        let leaf = "{}"
        let tooDeep = (0..<9).reduce(leaf) { json, _ in "{\"all\":[\(json)]}" }
        #expect(throws: FolioError.unreadableResponse) { _ = try decode(tooDeep) }
    }

    @Test func eventAndCandidateFormsExposeOnlyRelevantMergedFields() {
        let subject = WorkflowTriggerChoice.ConditionField(
            id: "event.subject_id", title: "Subject", valueType: "identifier")
        let source = WorkflowTriggerChoice(
            id: "source.failed", title: "Source failed", target: .event, conditionFields: [subject])
        let entry = WorkflowTriggerChoice(
            id: "document_note.created", title: "Note created", target: .document, conditionFields: [subject]
        )
        let candidate = WorkflowTriggerChoice(
            id: "ingest.received", title: "File arrives", target: .ingestCandidate, conditionFields: []
        )

        #expect(WorkflowConditionTree.Field.available(for: source).map(\.id) == ["event.subject_id"])
        #expect(WorkflowConditionTree.Field.available(for: entry).map(\.id) == ["event.subject_id"])
        let candidateIDs = WorkflowConditionTree.Field.available(for: candidate).map(\.id)
        #expect(candidateIDs.contains("filename_glob"))
        #expect(!candidateIDs.contains("tag_id"))
        #expect(!candidateIDs.contains("event.subject_id"))
    }

    @Test func sourceSnapshotCatalogOffersSourceKindAndComparisonsForEmailFields() {
        let choice = WorkflowTriggerChoice(
            id: "ingest.received", title: "File arrives", target: .ingestCandidate, conditionFields: [])
        let ids = Set(WorkflowConditionTree.Field.available(for: choice).map(\.id))
        #expect(
            ids.isSuperset(of: [
                "source.kind", "source.folder_path_glob", "source.email_from", "source.email_to",
                "source.email_subject", "source.email_attachment_glob", "source.email_mailbox_glob",
            ]))
    }

    @Test func editingAnEmailFilterPreservesSiblingsAndOtherGroups() throws {
        let from = WorkflowConditionTree.Field(id: "source.email_from", title: "Email sender")
        let to = WorkflowConditionTree.Field(id: "source.email_to", title: "Email recipient")
        var tree = try decode(
            """
            {"any":[],"all":[{"source":{"kind":"email","email_from":{"regex":"@example[.]test$"},
            "email_to":{"glob":"team?@example.test"},"email_mailbox_glob":"INBOX/*"},
            "event":{"actor":{"type":"system"}}}]}
            """
        )
        let saved = try #require(tree[.all]?.first)
        #expect(saved.textMatch(for: from) == .init(mode: .regex, pattern: "@example[.]test$"))

        tree[.all]?[0].setTextMatch("invoices@", mode: .contains, for: from)
        tree[.all]?[0].setTextMatch("", mode: .glob, for: to)
        let expected = try decode(
            """
            {"any":[],"all":[{"source":{"kind":"email","email_from":{"contains":"invoices@"},
            "email_mailbox_glob":"INBOX/*"},"event":{"actor":{"type":"system"}}}]}
            """
        )
        #expect(try tree.payload() == expected.payload())
    }

    @Test func advertisedFieldsUseCatalogLabelsAndTypes() {
        let choice = WorkflowTriggerChoice(
            id: "document.uploaded", title: "Document uploaded", target: .document,
            conditionFields: [
                .init(id: "mime_type", title: "Type at upload", valueType: "text"),
                .init(id: "tag_id", title: "Tag at upload", valueType: "identifier"),
                .init(id: "filename_glob", title: "Unexpected type", valueType: "boolean"),
            ]
        )
        let fields = WorkflowConditionTree.Field.available(for: choice)
        #expect(fields.map(\.id) == ["mime_type", "tag_id"])
        #expect(fields.map(\.title) == ["Type at upload", "Tag at upload"])
        #expect(fields.map(\.isIdentifier) == [false, true])
    }

    @Test func editsTheAdvertisedSourceWorkflowIDInsideAConditionGroup() throws {
        let choice = WorkflowTriggerChoice(
            id: "workflow_run.failed", title: "When a workflow run fails", target: .event,
            conditionFields: [
                .init(id: "event.subject_id", title: "Subject ID", valueType: "identifier"),
                .init(id: "event.workflow_id", title: "Source workflow ID", valueType: "identifier"),
            ]
        )
        let fields = WorkflowConditionTree.Field.available(for: choice)
        let filter = try #require(fields.first { $0.id == "event.workflow_id" })
        #expect(fields.map(\.id) == ["event.subject_id", "event.workflow_id"])
        #expect(filter.title == "Source workflow ID")
        var tree = try decode(#"{"all":[{"event":{"actor":{"type":"system"}}}]}"#)
        tree[.all]?[0][filter] = "missing ID"
        #expect(tree.validationMessage(fields: fields) == "Enter a valid UUID for Source workflow ID.")
        let id = "d328c55a-1bf3-4590-b878-ad432191caba"
        tree[.all]?[0][filter] = id
        #expect(tree.validationMessage(fields: fields) == nil)
        let expected = try decode(
            """
            {"all":[{"event":{"actor":{"type":"system"},"workflow_id":"\(id)"}}]}
            """)
        #expect(try tree.payload() == expected.payload())
    }

    @Test func processingFiltersUseAdvertisedTypesAtEveryDepth() throws {
        let choice = WorkflowTriggerChoice(
            id: "document.processing.ocr.completed", title: "Text extraction completed", target: .document,
            conditionFields: [
                .init(id: "data.has_text", title: "Text found", valueType: "boolean"),
                .init(id: "data.attempt", title: "Attempt", valueType: "integer"),
                .init(id: "data.run_id", title: "Processing run", valueType: "identifier"),
                .init(id: "data.retry_at", title: "Unexpected type", valueType: "text"),
                .init(id: "data.future_filter", title: "Future filter", valueType: "boolean"),
            ]
        )
        let fields = WorkflowConditionTree.Field.available(for: choice)
        #expect(
            fields.map(\.id) == ["event.processing.has_text", "event.processing.attempt", "event.processing.run_id"])
        let found = try #require(fields.first { $0.id == "event.processing.has_text" })
        let attempt = try #require(fields.first { $0.id == "event.processing.attempt" })
        let run = try #require(fields.first { $0.id == "event.processing.run_id" })
        var tree = try decode(
            #"""
            {"event":{"processing":{"has_text":true}},"all":[{"event":{"processing":{"attempt":1},
            "actor":{"type":"system"}}}]}
            """#
        )
        #expect(tree[found] == "true")
        #expect(tree[.all]?[0][attempt] == "1")
        tree[found] = "false"
        tree[.all]?[0][attempt] = "4294967296"
        #expect(tree.validationMessage(fields: fields) == "Enter a valid processing attempt.")
        tree[.all]?[0][attempt] = "4294967295"
        tree[run] = "unknown"
        #expect(tree.validationMessage(fields: fields) == "Enter a valid UUID for Processing run.")
        tree[run] = ""
        #expect(tree.validationMessage(fields: fields) == nil)
        #expect(!tree.hasAdditionalFilters(fields: fields, supportsActor: false))
        let expected = try decode(
            #"""
            {"event":{"processing":{"has_text":false}},"all":[{"event":{"processing":{"attempt":4294967295},
            "actor":{"type":"system"}}}]}
            """#
        )
        #expect(try tree.payload() == expected.payload())
        let reopened = try WorkflowConditionTree(tree.payload())
        #expect(reopened[found] == "false")
        #expect(reopened[.all]?[0][attempt] == "4294967295")

        tree[found] = ""
        tree[.all]?[0][attempt] = ""
        let cleared = try decode(#"{"all":[{"event":{"actor":{"type":"system"}}}]}"#)
        #expect(try tree.payload() == cleared.payload())
    }

    @Test func ingestionAndProcessingConditionsStayInTheirOwnPayloadFamilies() throws {
        let source = WorkflowTriggerChoice.ConditionField(
            id: "data.source_id", title: "Source ID", valueType: "identifier")
        let reason = WorkflowTriggerChoice.ConditionField(
            id: "data.reason_code", title: "Reason", valueType: "text")
        let failure = WorkflowTriggerChoice(
            id: "ingest.failed", title: "Item import failed", target: .event,
            conditionFields: [source, reason, .init(id: "data.retryable", title: "Retryable", valueType: "boolean")])
        let recovery = WorkflowTriggerChoice(
            id: "source.recovered", title: "Source recovered", target: .event, conditionFields: [source])
        let processing = WorkflowTriggerChoice(
            id: "document.processing.ocr.failed", title: "Text extraction failed", target: .document,
            conditionFields: [reason])

        let sourceChoices = try #require(
            WorkflowTriggerChoice.matching([failure.id, recovery.id], in: [failure, recovery]))
        #expect(WorkflowConditionTree.Field.available(for: sourceChoices).map(\.id) == ["event.ingest.source_id"])
        for selection in [[failure.id, processing.id], [processing.id, failure.id]] {
            let mixed = try #require(WorkflowTriggerChoice.matching(selection, in: [failure, processing]))
            #expect(WorkflowConditionTree.Field.available(for: mixed).isEmpty)
        }
    }

    @Test func editingIngestionPredicatesPreservesFalseAndUnrelatedNestedData() throws {
        let choice = WorkflowTriggerChoice(
            id: "ingest.failed", title: "Item import failed", target: .event,
            conditionFields: [
                .init(id: "data.source_id", title: "Source ID", valueType: "identifier"),
                .init(id: "data.source_kind", title: "Source kind", valueType: "text"),
                .init(id: "data.reason_code", title: "Reason", valueType: "text"),
                .init(id: "data.retryable", title: "Retryable", valueType: "boolean"),
            ]
        )
        let fields = WorkflowConditionTree.Field.available(for: choice)
        let source = try #require(fields.first { $0.id == "event.ingest.source_id" })
        let kind = try #require(fields.first { $0.id == "event.ingest.source_kind" })
        let reason = try #require(fields.first { $0.id == "event.ingest.reason_code" })
        let retry = try #require(fields.first { $0.id == "event.ingest.retryable" })
        #expect(retry.isBoolean)
        #expect(kind.isSourceKind)
        let original = """
            {"all":[{"event":{"ingest":{"source_kind":"email","outcome":"failed","retryable":true},
            "actor":{"type":"system"}}}],"any":[]}
            """
        var tree = try decode(original)
        var branch = try #require(tree[.all]?.first)
        #expect(branch[kind] == "email")
        #expect(branch[retry] == "true")
        branch[source] = "unknown"
        #expect(branch.validationMessage(fields: fields) == "Enter a valid UUID for Source ID.")
        branch[source] = ""
        branch[reason] = String(repeating: "é", count: 33)
        #expect(branch.validationMessage(fields: fields) == "Use 64 bytes or fewer for the ingestion reason.")
        branch[reason] = " unreadable_attachment "
        branch[retry] = "false"
        tree[.all]?[0] = branch
        #expect(tree.validationMessage(fields: fields) == nil)
        let expected = try decode(
            """
            {"all":[{"event":{"ingest":{"source_kind":"email","outcome":"failed","retryable":false,
            "reason_code":"unreadable_attachment"},"actor":{"type":"system"}}}],"any":[]}
            """)
        #expect(try tree.payload() == expected.payload())
        let reopened = try WorkflowConditionTree(tree.payload())
        #expect(reopened[.all]?.first?[retry] == "false")
    }

    @Test func processingRetryTimeKeepsTimestampPrecision() throws {
        let choice = WorkflowTriggerChoice(
            id: "document.processing.ocr.retry_scheduled", title: "Text extraction retry scheduled", target: .document,
            conditionFields: [.init(id: "data.retry_at", title: "Retry time", valueType: "timestamp")]
        )
        let fields = WorkflowConditionTree.Field.available(for: choice)
        let retry = try #require(fields.first)
        var tree = WorkflowConditionTree()
        tree[retry] = "2040-01-03"
        #expect(tree.validationMessage(fields: fields) == "Enter a valid timestamp for Retry time.")
        tree[retry] = "2040-01-03T09:30:00.123456789Z"
        #expect(tree.validationMessage(fields: fields) == nil)
        let reopened = try WorkflowConditionTree(tree.payload())
        #expect(reopened[retry] == "2040-01-03T09:30:00.123456789Z")
        let expected = try decode(#"{"event":{"processing":{"retry_at":"2040-01-03T09:30:00.123456789Z"}}}"#)
        #expect(try tree.payload() == expected.payload())
    }

    @Test func olderSnapshotCatalogKeepsKnownFilters() {
        let choice = WorkflowTriggerChoice(
            id: "document.uploaded", title: "Document uploaded", target: .document, conditionFields: []
        )
        #expect(WorkflowConditionTree.Field.available(for: choice).map(\.id).contains("filename_glob"))

        let unknown = WorkflowTriggerChoice(
            id: "document.future_event", title: "A future event", target: .document, conditionFields: []
        )
        #expect(WorkflowConditionTree.Field.available(for: unknown).isEmpty)
    }

    @Test func switchingActorKeepsOtherEventPredicates() throws {
        var tree = try decode(
            """
            {"event":{"actor":{"type":"user","user_id":"editor"},"subject_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"}}
            """
        )
        #expect(tree.actor == .user)
        #expect(tree.actorUserID == "editor")
        tree.actor = .system
        let system = try decode(
            """
            {"event":{"actor":{"type":"system"},"subject_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"}}
            """
        )
        #expect(try tree.payload() == system.payload())
        tree.actor = .user
        #expect(tree.validationMessage(fields: [], supportsActor: true) == "Enter the actor's user ID.")
        tree.actorUserID = "owner"
        #expect(tree.validationMessage(fields: [], supportsActor: true) == nil)
    }

    private func decode(_ json: String) throws -> WorkflowConditionTree {
        try WorkflowConditionTree(JSONDecoder().decode(OpenAPIObjectContainer.self, from: Data(json.utf8)))
    }
}
