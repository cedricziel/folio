import FolioKit
import Foundation
import SwiftUI

struct WorkflowLifecycleFixture: View {
    @State private var service = WorkflowLifecycleFixtureService()

    var body: some View {
        WorkflowLifecycleView(
            vaultID: WorkflowLifecycleFixtureService.vaultID, service: service,
            canManage: !ProcessInfo.processInfo.arguments.contains("--read-only")
        )
    }
}

@MainActor final class WorkflowLifecycleFixtureService: WorkflowLifecycleService {
    static let vaultID = UUID(uuidString: "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff")!
    private static let workflowID = UUID(uuidString: "00000000-0000-4000-8000-000000000001")!
    private let date = Date(timeIntervalSince1970: 1_793_620_800)
    private var deleted = false
    private var failList = ProcessInfo.processInfo.arguments.contains("--initial-load-error")
    private var failHistory = ProcessInfo.processInfo.arguments.contains("--workflow-history-error")
    private var failDelete = ProcessInfo.processInfo.arguments.contains("--workflow-delete-error")

    func workflowConfigurations(vaultID _: UUID, deleted: Bool) async throws -> [WorkflowConfigurationSummary] {
        if failList {
            failList = false
            throw FolioError.unreachable
        }
        return self.deleted == deleted ? [configuration] : []
    }

    func workflowHistory(vaultID _: UUID, workflowID _: UUID) async throws -> [WorkflowExecutionSummary] {
        if failHistory {
            failHistory = false
            throw FolioError.unreachable
        }
        return [
            WorkflowExecutionSummary(
                id: UUID(uuidString: "00000000-0000-4000-8000-000000000002")!,
                vaultID: Self.vaultID, workflowID: Self.workflowID, event: "document.uploaded",
                status: "failed", attempts: 3, completedActions: 1,
                lastError: "Receiver unavailable", responseStatus: 503, createdAt: date, finishedAt: date
            )
        ]
    }

    func deleteWorkflow(vaultID _: UUID, workflowID _: UUID) async throws {
        if failDelete {
            failDelete = false
            throw FolioError.unreachable
        }
        deleted = true
    }

    func restoreWorkflow(vaultID _: UUID, workflowID _: UUID) async throws -> WorkflowConfigurationSummary {
        deleted = false
        return configuration
    }

    private var configuration: WorkflowConfigurationSummary {
        WorkflowConfigurationSummary(
            id: Self.workflowID, vaultID: Self.vaultID, name: "Invoice notifications", enabled: true,
            trigger: "document.uploaded", createdAt: date, updatedAt: date, deletedAt: deleted ? date : nil,
            createdBy: "Taylor", updatedBy: "Morgan"
        )
    }
}
