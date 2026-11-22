import Foundation

extension LiveFolioService: WorkflowLifecycleService {
    public func workflowConfigurations(vaultID: UUID, deleted: Bool) async throws -> [WorkflowConfigurationSummary] {
        let data = try await managementRequest(
            path: "api/workflows?deleted=\(deleted)", vaultID: vaultID
        )
        let rows = try Self.workflowDecoder().decode([WorkflowConfigurationSummary].self, from: data)
        guard rows.allSatisfy({ $0.vaultID == vaultID && ($0.deletedAt != nil) == deleted }) else {
            throw FolioError.unreadableResponse
        }
        return rows
    }

    public func workflowHistory(vaultID: UUID, workflowID: UUID) async throws -> [WorkflowExecutionSummary] {
        let data = try await managementRequest(
            path: "api/workflows/\(workflowID.uuidString.lowercased())/runs?limit=50", vaultID: vaultID
        )
        let rows = try Self.workflowDecoder().decode([WorkflowExecutionSummary].self, from: data)
        guard rows.allSatisfy({ $0.vaultID == vaultID && $0.workflowID == workflowID }) else {
            throw FolioError.unreadableResponse
        }
        return rows
    }

    public func deleteWorkflow(vaultID: UUID, workflowID: UUID) async throws {
        _ = try await managementRequest(
            path: "api/workflows/\(workflowID.uuidString.lowercased())", method: "DELETE", vaultID: vaultID
        )
    }

    public func restoreWorkflow(vaultID: UUID, workflowID: UUID) async throws -> WorkflowConfigurationSummary {
        let data = try await managementRequest(
            path: "api/workflows/\(workflowID.uuidString.lowercased())/restore", method: "POST", vaultID: vaultID
        )
        let row = try Self.workflowDecoder().decode(WorkflowConfigurationSummary.self, from: data)
        guard row.vaultID == vaultID, row.id == workflowID, row.deletedAt == nil else {
            throw FolioError.unreadableResponse
        }
        return row
    }

    private static func workflowDecoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            try LenientDateTranscoder().decode(decoder.singleValueContainer().decode(String.self))
        }
        return decoder
    }
}
