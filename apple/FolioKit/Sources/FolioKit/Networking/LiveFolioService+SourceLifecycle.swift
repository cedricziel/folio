import Foundation

extension LiveFolioService: SourceLifecycleService {
    public func sourceActivity(vaultID: UUID, deleted: Bool) async throws -> [SourceActivity] {
        let data = try await managementRequest(path: "api/sources" + (deleted ? "?deleted=true" : ""), vaultID: vaultID)
        let rows = try sourceDecoder().decode([SourceLifecycleWire].self, from: data)
        return try rows.map { row in
            let source = try row.source(vaultID: vaultID)
            guard (source.deletedAt != nil) == deleted else { throw FolioError.unreadableResponse }
            return source
        }
    }

    public func setSourceEnabled(vaultID: UUID, sourceID: UUID, enabled: Bool) async throws -> SourceActivity {
        let path = "api/sources/\(sourceID.uuidString.lowercased())"
        let current = try await managementRequest(path: path, vaultID: vaultID)
        let source = try decodeSource(current, vaultID: vaultID, sourceID: sourceID)
        guard source.supportsLifecycle, source.deletedAt == nil else { throw FolioError.unreadableResponse }
        guard let json = try JSONSerialization.jsonObject(with: current) as? [String: Any],
            let config = json["config"] as? [String: Any]
        else { throw FolioError.unreadableResponse }
        // Fetch settings immediately before replacing them so a status toggle retains the current masked secrets.
        let body = try JSONSerialization.data(withJSONObject: [
            "name": source.name, "enabled": enabled, "config": config,
        ])
        let data = try await managementRequest(path: path, method: "PUT", body: body, vaultID: vaultID)
        let updated = try decodeSource(data, vaultID: vaultID, sourceID: sourceID)
        guard updated.supportsLifecycle, updated.deletedAt == nil, updated.enabled == enabled else {
            throw FolioError.unreadableResponse
        }
        return updated
    }

    public func trashSource(vaultID: UUID, sourceID: UUID) async throws {
        _ = try await managementRequest(
            path: "api/sources/\(sourceID.uuidString.lowercased())", method: "DELETE", vaultID: vaultID)
    }

    public func restoreSource(vaultID: UUID, sourceID: UUID) async throws -> SourceActivity {
        let data = try await managementRequest(
            path: "api/sources/\(sourceID.uuidString.lowercased())/restore", method: "POST", vaultID: vaultID)
        let restored = try decodeSource(data, vaultID: vaultID, sourceID: sourceID)
        guard restored.supportsLifecycle, restored.deletedAt == nil else { throw FolioError.unreadableResponse }
        return restored
    }

    private func decodeSource(_ data: Data, vaultID: UUID, sourceID: UUID) throws -> SourceActivity {
        let source = try sourceDecoder().decode(SourceLifecycleWire.self, from: data).source(vaultID: vaultID)
        guard source.id == sourceID else { throw FolioError.unreadableResponse }
        return source
    }

    private func sourceDecoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        decoder.dateDecodingStrategy = .custom { decoder in
            try LenientDateTranscoder().decode(decoder.singleValueContainer().decode(String.self))
        }
        return decoder
    }
}

private struct SourceLifecycleWire: Decodable {
    struct Status: Decodable {
        let lastError: String?
        let lastRunAt: Date?
    }
    let id: UUID
    let vaultId: UUID
    let name: String
    let enabled: Bool
    let origin: String?
    let status: Status?
    let createdAt: Date?
    let updatedAt: Date?
    let deletedAt: Date?
    let createdBy: String?
    let updatedBy: String?

    func source(vaultID: UUID) throws -> SourceActivity {
        guard vaultId == vaultID else { throw FolioError.unreadableResponse }
        let health: SourceActivity.Health
        if !enabled {
            health = .off
        } else if status?.lastError != nil {
            health = .unavailable
        } else if status?.lastRunAt != nil {
            health = .available
        } else {
            health = .notChecked
        }
        return SourceActivity(
            id: id, vaultID: vaultID, name: name, health: health, lastCheckedAt: status?.lastRunAt,
            origin: origin.flatMap(SourceActivity.Origin.init(rawValue:)) ?? .unknown, enabled: enabled,
            createdAt: createdAt, updatedAt: updatedAt, deletedAt: deletedAt,
            createdBy: createdBy, updatedBy: updatedBy)
    }
}
