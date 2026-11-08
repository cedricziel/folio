import AppIntents
import Foundation

/// A document as App Intents, Spotlight and on-screen awareness name it: the document and the vault that holds it,
/// so an intent running outside the app still reaches the right vault. Identifiers saved before vaults existed
/// carry no vault and resolve in the server's default vault.
public struct DocumentEntityID: Hashable, Sendable, EntityIdentifierConvertible {
    public let documentID: Document.ID
    public let vaultID: UUID?

    public init(documentID: Document.ID, vaultID: UUID? = nil) {
        self.documentID = documentID
        self.vaultID = vaultID
    }

    /// The same string as the document's Spotlight identifier, or the bare document id without a vault.
    public var entityIdentifierString: String {
        guard let vaultID else { return documentID.uuidString.lowercased() }
        return DocumentLink(documentID: documentID, vaultID: vaultID).spotlightIdentifier
    }

    public static func entityIdentifier(for entityIdentifierString: String) -> DocumentEntityID? {
        if let link = DocumentLink(spotlightIdentifier: entityIdentifierString) {
            return DocumentEntityID(documentID: link.documentID, vaultID: link.vaultID)
        }
        return UUID(uuidString: entityIdentifierString).map { DocumentEntityID(documentID: $0) }
    }

    /// The document's page in the web app, which asks for its own sign-in.
    public func webURL(server: URL) -> URL {
        let document = "documents/\(documentID.uuidString.lowercased())"
        guard let vaultID else { return server.appending(path: document) }
        return server.appending(path: "vaults/\(vaultID.uuidString.lowercased())/\(document)")
    }
}
