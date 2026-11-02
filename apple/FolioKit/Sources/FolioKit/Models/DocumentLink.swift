import Foundation

public struct DocumentLink: Equatable, Sendable {
    public let documentID: Document.ID
    public let vaultID: UUID

    public init(documentID: Document.ID, vaultID: UUID) {
        self.documentID = documentID
        self.vaultID = vaultID
    }

    public var spotlightIdentifier: String {
        "folio:document:\(vaultID.uuidString.lowercased()):\(documentID.uuidString.lowercased())"
    }

    public init?(spotlightIdentifier: String) {
        let parts = spotlightIdentifier.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 4, parts[0] == "folio", parts[1] == "document",
            let vaultID = UUID(uuidString: String(parts[2])),
            let documentID = UUID(uuidString: String(parts[3]))
        else { return nil }
        self.init(documentID: documentID, vaultID: vaultID)
    }
}
