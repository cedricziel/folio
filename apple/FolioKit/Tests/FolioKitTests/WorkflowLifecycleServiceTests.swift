import FolioAPI
import Foundation
import OpenAPIURLSession
import Testing

@testable import FolioKit

@Suite(.serialized) struct WorkflowLifecycleServiceTests {
    private let vaultID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
    private let workflowID = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!

    @Test func expandedCatalogOffersSavedViewTriggersTheClientCanSave() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            let query = request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
            if query?.queryItems?.contains(.init(name: "include_extended", value: "true")) == true {
                return (
                    200,
                    """
                    [{"event_type":"saved_view.updated","label":"When a shared saved view changes",
                    "subject_type":"saved_view","target":"event","condition_fields":[]},
                    {"event_type":"vault_member_policy.updated","label":"When a policy changes",
                    "subject_type":"vault_member_policy","target":"event","condition_fields":[]}]
                    """
                )
            }
            return (200, "[]")
        }
        let choices = try await service().workflowTriggerChoices(vaultID: vaultID)
        #expect(choices.map(\.id) == ["saved_view.updated"])
    }

    @Test func trashListUsesExplicitVaultAndIncludesLifecycleAttribution() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows")
            #expect(request.url?.query == "deleted=true")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, "[\(workflowJSON(deleted: true))]")
        }
        let rows = try await service().workflowConfigurations(vaultID: vaultID, deleted: true)
        #expect(rows.count == 1)
        #expect(rows.first?.createdBy == "creator")
        #expect(rows.first?.updatedBy == "editor")
        #expect(rows.first?.stateTitle == "In Trash")
        #expect(rows.first?.deletedAt != nil)
        #expect(rows.first?.revision == 1)
    }

    @Test func retainedHistoryUsesSelectedVaultAndMapsFailedRunDetails() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())/runs")
            #expect(request.url?.query == "limit=50")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (
                200,
                """
                [{"id":"99999999-8888-4777-8666-555555555555","vault_id":"\(vaultID)",
                "workflow_id":"\(workflowID)","event":"document.uploaded","document_id":null,
                "status":"failed","attempts":3,"completed_actions":1,"last_error":"Receiver unavailable",
                "response_status":503,"created_at":"2026-11-02T12:34:56.123456789Z","finished_at":"2026-11-02T12:35:00Z"}]
                """
            )
        }
        let rows = try await service().workflowHistory(vaultID: vaultID, workflowID: workflowID)
        #expect(rows.first?.statusTitle == "Failed")
        #expect(rows.first?.attempts == 3)
        #expect(rows.first?.completedActions == 1)
        #expect(rows.first?.responseStatus == 503)
        #expect(rows.first?.lastError == "Receiver unavailable")
    }

    @Test func deletionUsesTheLifecycleEndpointWithoutARequestBody() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())")
            #expect(request.httpMethod == "DELETE")
            #expect(request.httpBody == nil)
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (204, "")
        }
        try await service().deleteWorkflow(vaultID: vaultID, workflowID: workflowID)
    }

    @Test func restorationReturnsTheActiveConfiguration() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())/restore")
            #expect(request.httpMethod == "POST")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, workflowJSON(deleted: false))
        }
        let restored = try await service().restoreWorkflow(vaultID: vaultID, workflowID: workflowID)
        #expect(restored.id == workflowID)
        #expect(restored.deletedAt == nil)
    }

    @Test func anotherVaultCannotSupplyConfigurationRows() async {
        LifecycleURLProtocol.handler = { _ in
            (200, "[\(workflowJSON(deleted: false))]")
        }
        await #expect(throws: FolioError.unreadableResponse) {
            try await service().workflowConfigurations(vaultID: UUID(), deleted: false)
        }
    }

    @Test func restoreRejectsAResponseThatIsStillInTrash() async {
        LifecycleURLProtocol.handler = { _ in (200, workflowJSON(deleted: true)) }
        await #expect(throws: FolioError.unreadableResponse) {
            try await service().restoreWorkflow(vaultID: vaultID, workflowID: workflowID)
        }
    }

    @Test func serverOwnerRestrictionsArePreserved() async {
        LifecycleURLProtocol.handler = { _ in (403, "{}") }
        await #expect(throws: FolioError.server(status: 403, message: nil)) {
            try await service().workflowConfigurations(vaultID: vaultID, deleted: false)
        }
    }

    @Test func expandedCatalogReturnsOnlyTriggersThisPickerCanSave() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/folio/api/workflows/triggers")
            #expect(request.url?.query == "include_extended=true")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (
                200,
                """
                [{"event_type":"saved_view.updated","label":"When a shared saved view is updated",
                "subject_type":"saved_view","target":"event","condition_fields":[]},
                {"event_type":"future.event","label":"When a future event happens",
                "subject_type":"future","target":"event","condition_fields":[]}]
                """
            )
        }
        let choices = try await service().workflowTriggerChoices(vaultID: vaultID)
        #expect(choices.map(\.id) == ["saved_view.updated"])
    }

    @Test(arguments: [1, 2])
    func creatingAWorkflowSendsTheOrderedSelectionOrTheLegacySingleTrigger(count: Int) async throws {
        let choices = [
            WorkflowTriggerChoice(id: "document.uploaded", title: "Upload", target: .document, conditionFields: []),
            WorkflowTriggerChoice(id: "document_note.created", title: "Note", target: .document, conditionFields: []),
        ]
        let selected = Array(choices.prefix(count))
        let ids = selected.map(\.id)
        var draft = WorkflowActionDraft()
        var hook = WorkflowActionDraft.Row(kind: .webhook)
        hook.url = "https://receiver.test/hook"
        draft.rows = [hook]
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "POST")
            #expect(request.url?.path == "/folio/api/workflows")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            guard let data = LifecycleURLProtocol.body(for: request),
                let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            else { return (400, "{}") }
            #expect(body["enabled"] as? Bool == false)
            #expect(body["name"] as? String == "New rule")
            if count == 1 {
                #expect(body["trigger"] as? String == ids.first)
                #expect(body["triggers"] == nil)
            } else {
                #expect(body["trigger"] == nil)
                #expect(body["triggers"] as? [String] == ids)
            }
            return (201, workflowJSON(deleted: false, enabled: false, triggers: count == 1 ? nil : ids))
        }
        let created = try await service().createWorkflow(
            vaultID: vaultID, name: " New rule ", choices: selected, actions: draft)
        #expect(created.selectedTriggers == ids)
    }

    @Test func generatedCatalogAndUpdateRoundTripEntryAndIngestionChoices() async throws {
        let entryIDs = [
            "document_note.created", "document_note.updated", "document_note.deleted",
            "document_comment.created", "document_comment.updated", "document_comment.deleted",
        ]
        let ingestionIDs = [
            "ingest.imported", "ingest.skipped", "ingest.duplicate", "ingest.failed",
            "source.failed", "source.recovered",
        ]
        let choiceIDs = entryIDs + ingestionIDs
        let field = WorkflowTriggerChoice.ConditionField(
            id: "event.subject_id", title: "Subject ID", valueType: "identifier")
        let definitions: [[String: Any]] = choiceIDs.map { id in
            [
                "event_type": id, "label": "Catalog label for \(id)",
                "subject_type": String(id.split(separator: ".")[0]),
                "target": entryIDs.contains(id) ? "document" : "event",
                "condition_fields": [["key": field.id, "label": field.title, "value_type": field.valueType]],
            ]
        }
        let catalog = try JSONSerialization.data(withJSONObject: definitions)
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == "/folio/api/workflows/triggers")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, String(decoding: catalog, as: UTF8.self))
        }
        let live = service()
        let choices = try await live.workflowTriggerChoices(vaultID: vaultID)
        #expect(choices.map(\.id) == choiceIDs)
        #expect(choices.map(\.target) == entryIDs.map { _ in .document } + ingestionIDs.map { _ in .event })
        #expect(choices.allSatisfy { $0.title == "Catalog label for \($0.id)" && $0.conditionFields == [field] })

        let path = "/folio/api/workflows/\(workflowID.uuidString.lowercased())"
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == path)
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (200, workflowJSON(deleted: false))
        }
        let summary = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        #expect(summary.revision == 1)

        for choice in choices {
            LifecycleURLProtocol.handler = { request in
                #expect(request.httpMethod == "PATCH")
                #expect(request.url?.path == path + "/trigger")
                #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
                guard let data = LifecycleURLProtocol.body(for: request),
                    let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
                else { return (400, "{}") }
                #expect(body["trigger"] as? String == choice.id)
                #expect(body["expected_revision"] as? Int64 == 1)
                #expect(Set(body.keys) == ["trigger", "expected_revision"])
                return (200, workflowJSON(deleted: false, trigger: choice.id, revision: 2))
            }
            let updated = try await live.changeWorkflowTrigger(summary, to: choice.id)
            #expect(updated.trigger == choice.id)
            #expect(updated.vaultID == vaultID)
            #expect(updated.revision == 2)
        }
    }

    @Test func aStaleTriggerEditCanReloadAndRetryTheNewRevision() async throws {
        let live = service()
        let path = "/folio/api/workflows/\(workflowID.uuidString.lowercased())"
        LifecycleURLProtocol.handler = { _ in (200, workflowJSON(deleted: false)) }
        let stale = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "PATCH")
            #expect(request.url?.path == path + "/trigger")
            return (409, #"{"error":"Workflow changed. Reload it before editing."}"#)
        }
        await #expect(throws: FolioError.server(status: 409, message: "Workflow changed. Reload it before editing.")) {
            try await live.changeWorkflowTrigger(stale, to: "document_note.created")
        }
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.url?.path == path)
            return (200, workflowJSON(deleted: false, revision: 2, enabled: false))
        }
        let latest = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        #expect(!latest.enabled)
        #expect(latest.revision == 2)
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "PATCH")
            #expect(request.url?.path == path + "/trigger")
            guard let data = LifecycleURLProtocol.body(for: request),
                let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            else { return (400, "{}") }
            #expect(body["expected_revision"] as? Int64 == 2)
            #expect(body["enabled"] == nil)
            return (200, workflowJSON(deleted: false, trigger: "document_note.created", revision: 3, enabled: false))
        }
        let saved = try await live.changeWorkflowTrigger(latest, to: "document_note.created")
        #expect(!saved.enabled)
        #expect(saved.revision == 3)
    }

    @Test func anOlderServerKeepsTheListReadableWithoutOfferingUnconditionalTriggerEdits() async throws {
        let live = service()
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            return (200, workflowJSON(deleted: false, revision: nil))
        }
        let workflow = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        #expect(workflow.revision == nil)
        await #expect(throws: FolioError.unreadableResponse) {
            try await live.changeWorkflowTrigger(workflow, to: "document_note.created")
        }
    }

    @Test func generatedDefinitionEditSendsTheExactRevisionAndSurfacesConflicts() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [LifecycleURLProtocol.self]
        let client = Client(
            serverURL: URL(string: "https://workflow-lifecycle.test/folio")!,
            transport: URLSessionTransport(configuration: .init(session: URLSession(configuration: config))),
            middlewares: [VaultScopeMiddleware(vaultID: vaultID)]
        )
        let path = Operations.UpdateWorkflowDefinition.Input.Path(id: workflowID.uuidString.lowercased())
        let draft = try JSONDecoder().decode(
            Components.Schemas.WorkflowInput.self, from: Data(workflowJSON(deleted: false).utf8)
        )
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "PUT")
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())/definition")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            #expect(request.url?.query == "expected_revision=9007199254740993")
            return (409, #"{"error":"Workflow changed. Reload it before editing."}"#)
        }
        let result = try await client.updateWorkflowDefinition(
            path: path, query: .init(expectedRevision: 9_007_199_254_740_993), body: .json(draft)
        )
        #expect(try result.conflict.body.json.error == "Workflow changed. Reload it before editing.")
    }

    @Test func fullEditorsRetryWithTheReloadedDisabledConfiguration() async throws {
        let live = service()
        LifecycleURLProtocol.handler = { request in
            if request.httpMethod == "GET" { return (200, workflowJSON(deleted: false)) }
            #expect(request.httpMethod == "PUT")
            #expect(request.url?.path.hasSuffix("/definition") == true)
            #expect(request.url?.query == "expected_revision=1")
            return (409, #"{"error":"The workflow changed."}"#)
        }
        let stale = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        await #expect(throws: FolioError.server(status: 409, message: "The workflow changed.")) {
            try await live.saveWorkflowSettings(stale, name: "Updated", enabled: true)
        }

        LifecycleURLProtocol.handler = { request in
            if request.httpMethod == "GET" { return (200, workflowJSON(deleted: false, revision: 2, enabled: false)) }
            #expect(request.httpMethod == "PUT")
            #expect(request.url?.query == "expected_revision=2")
            guard let bytes = LifecycleURLProtocol.body(for: request),
                let body = (try? JSONSerialization.jsonObject(with: bytes)) as? [String: Any]
            else { return (400, "{}") }
            #expect(body["enabled"] as? Bool == false)
            #expect((body["actions"] as? [[String: Any]])?.first?["secret"] as? String == "********")
            return (200, workflowJSON(deleted: false, revision: 3, enabled: false))
        }
        let latest = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        let tree = try await live.loadWorkflowConditions(latest)
        let saved = try await live.saveWorkflowConditions(latest, conditions: tree)
        #expect(saved.revision == 3)
        #expect(!saved.enabled)
    }

    @Test func orderedActionsAndCadenceUseTheSameGuardedDefinitionRoute() async throws {
        let live = service()
        let choice = WorkflowTriggerChoice(
            id: "document.uploaded", title: "When a document is added", target: .document, conditionFields: []
        )
        LifecycleURLProtocol.handler = { request in
            if request.httpMethod == "GET" { return (200, workflowJSON(deleted: false)) }
            #expect(request.httpMethod == "PUT")
            #expect(request.url?.path.hasSuffix("/definition") == true)
            #expect(request.url?.query == "expected_revision=1")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Workflow-Action-Sources") == "1,0")
            return (200, workflowJSON(deleted: false, revision: 2))
        }
        let initial = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        var draft = try await live.loadWorkflowActions(initial)
        draft.move(try #require(draft.rows.first?.id), by: 1)
        let ordered = try await live.saveWorkflowActions(initial, actions: draft, choice: choice)
        #expect(ordered.revision == 2)

        let scheduled = "workflow.scheduled"
        var schedule = WorkflowCadenceDraft()
        schedule.start = "2032-02-29T09:30:00.123456789Z"
        let cadence = try schedule.payload(for: scheduled)
        LifecycleURLProtocol.handler = { request in
            if request.httpMethod == "GET" {
                return (200, workflowJSON(deleted: false, trigger: scheduled, revision: 2))
            }
            #expect(request.httpMethod == "PUT")
            #expect(request.url?.path.hasSuffix("/definition") == true)
            #expect(request.url?.query == "expected_revision=2")
            guard let bytes = LifecycleURLProtocol.body(for: request),
                let body = (try? JSONSerialization.jsonObject(with: bytes)) as? [String: Any]
            else { return (400, "{}") }
            #expect((body["cadence"] as? [String: String])?["at"] == "2032-02-29T09:30:00.123456789Z")
            return (200, workflowJSON(deleted: false, trigger: scheduled, revision: 3))
        }
        let clock = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)
        let updated = try await live.saveWorkflowTriggers(clock, to: [scheduled], cadence: cadence)
        #expect(updated.revision == 3)
    }

    @Test func orderedTriggersUseTheGuardedDefinitionAndRetainActions() async throws {
        let original = ["document.uploaded", "document_note.created", "document_comment.created"]
        let reordered = ["document_comment.created", "document.uploaded", "document_note.created"]
        let initialJSON = workflowJSON(deleted: false, revision: 7, triggers: original)
        let updatedJSON = workflowJSON(deleted: false, trigger: reordered[0], revision: 8, triggers: reordered)
        let live = service()
        LifecycleURLProtocol.handler = { request in
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            if request.httpMethod == "GET" {
                return (200, initialJSON)
            }
            #expect(request.httpMethod == "PUT")
            #expect(request.url?.path == "/folio/api/workflows/\(workflowID.uuidString.lowercased())/definition")
            #expect(request.url?.query == "expected_revision=7")
            guard let data = LifecycleURLProtocol.body(for: request),
                let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
                let saved = (try? JSONSerialization.jsonObject(with: Data(initialJSON.utf8))) as? [String: Any]
            else { return (400, "{}") }
            #expect(body["trigger"] as? String == reordered[0])
            #expect(body["triggers"] as? [String] == reordered)
            #expect(body["conditions"] as? NSDictionary == saved["conditions"] as? NSDictionary)
            #expect(body["actions"] as? NSArray == saved["actions"] as? NSArray)
            return (200, updatedJSON)
        }
        let workflow = try await live.workflowConfiguration(vaultID: vaultID, workflowID: workflowID)

        let updated = try await live.saveWorkflowTriggers(workflow, to: reordered, cadence: nil)

        #expect(updated.selectedTriggers == reordered)
        #expect(updated.revision == 8)
    }

    @Test func actionTaxonomyUsesTheWorkflowVaultAndReturnsActiveEntries() async throws {
        LifecycleURLProtocol.handler = { request in
            #expect(request.url?.path == "/folio/api/tags")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            return (
                200,
                """
                [{"id":"11111111-2222-4333-8444-555555555555","vault_id":"\(vaultID)",
                "name":"Suppliers","created_at":"2026-11-02T12:34:56Z","updated_at":"2026-11-02T12:34:56Z",
                "deleted_at":null},
                {"id":"22222222-2222-4333-8444-555555555555","vault_id":"\(vaultID)",
                "name":"Old supplier","created_at":"2026-11-02T12:34:56Z","updated_at":"2026-11-02T12:34:56Z",
                "deleted_at":"2026-11-03T12:34:56Z"}]
                """
            )
        }

        let entries = try await service().workflowActionTaxonomy(.tags, vaultID: vaultID)

        #expect(entries.map(\.name) == ["Suppliers"])
    }

    @Test func actionTaxonomyRejectsEntriesFromAnotherVault() async {
        LifecycleURLProtocol.handler = { _ in
            (
                200,
                """
                [{"id":"11111111-2222-4333-8444-555555555555","vault_id":"99999999-8888-4777-8666-555555555555",
                "name":"Suppliers","created_at":"2026-11-02T12:34:56Z","updated_at":"2026-11-02T12:34:56Z"}]
                """
            )
        }

        await #expect(throws: FolioError.unreadableResponse) {
            try await service().workflowActionTaxonomy(.tags, vaultID: vaultID)
        }
    }

    @MainActor @Test func membersUseReadOnlyRoutesAndClearDataAfterRevocation() async throws {
        let live = service()
        let runID = UUID(uuidString: "99999999-8888-4777-8666-555555555555")!
        LifecycleURLProtocol.handler = { request in
            #expect(request.httpMethod == "GET")
            #expect(request.value(forHTTPHeaderField: "X-Folio-Vault") == vaultID.uuidString.lowercased())
            switch request.url?.path {
            case "/folio/api/workflows/summaries":
                return (
                    200,
                    """
                    [{"id":"\(workflowID)","vault_id":"\(vaultID)","name":"File invoices","enabled":true,
                    "trigger":"document.processed","action_count":2,"trigger_selection":[
                    {"event_type":"document.processed","label":"When a document has been read"},
                    {"event_type":"saved_view.updated","label":"When a shared saved view is updated"}],
                    "created_at":"2026-11-02T12:34:56Z","updated_at":"2026-11-02T12:35:00Z"}]
                    """
                )
            case "/folio/api/workflows/\(workflowID.uuidString.lowercased())/run-summaries":
                #expect(request.url?.query == "limit=50")
                return (
                    200,
                    """
                    [{"id":"\(runID)","vault_id":"\(vaultID)","workflow_id":"\(workflowID)",
                    "event":"document.uploaded","status":"succeeded","attempts":2,"completed_actions":2,
                    "created_at":"2026-11-02T12:34:56Z","finished_at":"2026-11-02T12:35:00Z"}]
                    """
                )
            default:
                Issue.record("Unexpected workflow request: \(request.httpMethod ?? "") \(request.url?.path ?? "")")
                return (404, "{}")
            }
        }
        let list = WorkflowLifecycleViewModel(vaultID: vaultID, service: live, canManage: false)
        let history = WorkflowHistoryViewModel(
            vaultID: vaultID, workflowID: workflowID, service: live, canManage: false)
        #expect(list.canRead)
        #expect(!list.canManage)
        await list.load()
        let summary = try #require(list.workflows.first)
        #expect(summary.id == workflowID)
        #expect(summary.actionCount == 2)
        #expect(summary.selectedTriggers == ["document.processed", "saved_view.updated"])
        #expect(
            list.triggerTitle(for: summary)
                == "When a document has been read · When a shared saved view is updated")
        #expect(summary.createdBy == nil)
        #expect(!(await list.moveToTrash(summary)))
        await list.load(deleted: true)
        #expect(!list.deleted)
        await history.load()
        let run = try #require(history.runs.first)
        #expect(run.id == runID)
        #expect(run.completedActions == 2)
        #expect(run.finishedAt != nil)
        #expect(run.lastError == nil)
        #expect(run.responseStatus == nil)

        LifecycleURLProtocol.handler = { _ in (404, "{}") }
        await list.load()
        await history.load()
        #expect(list.workflows.isEmpty)
        #expect(list.accessDenied)
        #expect(history.runs.isEmpty)
        #expect(history.accessDenied)
    }

    private func service() -> LiveFolioService {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [LifecycleURLProtocol.self]
        return LiveFolioService(
            baseURL: URL(string: "https://workflow-lifecycle.test/folio")!,
            session: URLSession(configuration: config), vaultID: UUID()
        )
    }

    private func workflowJSON(
        deleted: Bool, trigger: String = "document.uploaded", revision: Int64? = 1, enabled: Bool = true,
        triggers: [String]? = nil
    ) -> String {
        let selected = triggers.map { "[" + $0.map { "\"\($0)\"" }.joined(separator: ",") + "]" }
        let triggersField = selected.map { "\"triggers\":\($0)," } ?? ""
        let revisionField = revision.map { "\"revision\":\($0)," } ?? ""
        return """
            {"id":"\(workflowID)","vault_id":"\(vaultID)","name":"File invoices","enabled":\(enabled),\(revisionField)
            "trigger":"\(trigger)",\(triggersField)"conditions":{"any":[{"event":{"actor":{"type":"system"}}}],"none":[]},
            "actions":[{"type":"webhook","url":"https://receiver.test/hook","method":"POST","secret":"********",
            "headers":{"Authorization":"********"}},{"type":"webhook","url":"https://receiver.test/hook","method":"POST",
            "headers":{"X-Custom-Credential":"********"}}],
            "created_at":"2026-11-02T12:34:56.123456789Z","updated_at":"2026-11-02T12:35:00Z",
            "deleted_at":\(deleted ? "\"2026-11-02T12:36:00Z\"" : "null"),"created_by":"creator","updated_by":"editor"}
            """
    }
}

private final class LifecycleURLProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: ((URLRequest) -> (Int, String))?

    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "workflow-lifecycle.test"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        let (status, json) = Self.handler?(request) ?? (500, "{}")
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(json.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    static func body(for request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = stream.read(&buffer, maxLength: buffer.count)
            guard count >= 0 else { return nil }
            if count == 0 { return data }
            data.append(buffer, count: count)
        }
    }

    override func stopLoading() {}
}
